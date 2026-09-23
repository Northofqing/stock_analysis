//! 复权价格调整 — 纯算法模块
//!
//! 基于除权除息信息对 K 线价格进行前复权/后复权计算。
//! TDX 服务端 CMD_SECURITY_BARS 返回的是未复权原始数据,
//! 复权需在客户端侧完成。
//!
//! ## 公式 (中国A股标准除权除息公式)
//!
//! 除权除息日开盘参考价:
//!   P_ex = (P_close - D + P_rights × R_rights) / (1 + R_bonus + R_rights)
//!
//! 前复权因子: factor = P_ex / P_close
//!   所有除权日之前的 K 线价格 × cumulative_factor
//!
//! 后复权因子: factor = P_close / P_ex
//!   所有除权日当天及之后的 K 线价格 × cumulative_factor
//!
//! ## 参数单位
//!
//! | TDX 字段     | 单位         | 算法中使用    |
//! |-------------|-------------|-------------|
//! | fenhong     | 元/10股     | ÷10 = 元/股 |
//! | songzhuangu | 股/10股     | ÷10 = 比例  |
//! | peigu       | 股/10股     | ÷10 = 比例  |
//! | peigujia    | 元/股       | 直接使用     |

use crate::protocol::constants::FQ_PRICE_PRECISION;
use crate::protocol::types::{IndexBar, SecurityBar, XdXrInfo};
use std::collections::BTreeMap;

// ================================================================
// 复权因子计算接口 (独立于 K 线调整)
// ================================================================

/// 单个除权事件的因子详情
#[derive(Debug, Clone)]
pub struct FqFactorItem {
    /// 除权除息日期 (YYYYMMDD)
    pub date: u32,
    /// 前收盘价 (除权日前一交易日收盘价)
    pub close_before: f64,
    /// 前复权因子 (QFQ)
    pub qfq_factor: f64,
    /// 后复权因子 (HFQ = 1/QFQ)
    pub hfq_factor: f64,
    /// 分红 (元/股, 已从元/10股转换)
    pub div_per_share: f64,
    /// 送股比例 (已从股/10股转换)
    pub bonus_ratio: f64,
    /// 配股比例 (已从股/10股转换)
    pub rights_ratio: f64,
    /// 配股价 (元/股)
    pub rights_price: f64,
}

/// 复权因子计算结果
#[derive(Debug, Clone)]
pub struct FqFactorResult {
    /// 因子列表 (按日期升序)
    pub factors: Vec<FqFactorItem>,
    /// 累计前复权因子 (所有因子的乘积)
    pub cumulative_qfq: f64,
    /// 累计后复权因子 (= 1/cumulative_qfq)
    pub cumulative_hfq: f64,
}

/// 计算复权因子 (不修改 K 线数据)
///
/// 根据 XDXR 历史和上下文 K 线数据，计算并返回每个除权事件的复权因子。
/// 可用于:
/// - 验证复权精度
/// - 手动应用复权调整
/// - 导出因子表供外部使用
///
/// # 参数
///
/// - `xdxr_list`: 该股票的除权除息历史 (由 get_xdxr_info 获取)
/// - `bars`: 请求的 K 线数据 (按日期升序)
/// - `context_bars`: 额外的历史 K 线数据 (按日期升序)，用于查找除权日前收盘价
///
/// # 返回
///
/// `FqFactorResult` 包含每个事件的因子和累计因子。
/// 如果某个事件找不到前收盘价，该事件将被跳过。
pub fn calc_fq_factors(
    xdxr_list: &[XdXrInfo],
    bars: &[SecurityBar],
    context_bars: &[SecurityBar],
) -> FqFactorResult {
    let mut factors = Vec::new();

    // 1. 构建除权事件 (升序)
    let mut events: Vec<(u32, FactorParts)> = Vec::new();
    for xd in xdxr_list {
        if xd.category != 1 {
            continue;
        }
        let date_key = xd.year * 10000 + xd.month * 100 + xd.day;
        events.push((
            date_key,
            FactorParts {
                div_per_share: xd.fenhong.unwrap_or(0.0) / 10.0,
                bonus_ratio: xd.songzhuangu.unwrap_or(0.0) / 10.0,
                rights_ratio: xd.peigu.unwrap_or(0.0) / 10.0,
                rights_price: xd.peigujia.unwrap_or(0.0),
            },
        ));
    }
    events.sort_by_key(|e| e.0);

    // 2. 计算每个事件的因子
    let mut cumulative_qfq = 1.0;
    for &(date_key, parts) in &events {
        if let Some(p_close) = find_close_before_event(bars, context_bars, date_key) {
            let qfq_factor = calc_qfq_factor(p_close, &parts);
            let hfq_factor = if qfq_factor.abs() > 1e-10 {
                1.0 / qfq_factor
            } else {
                1.0
            };

            cumulative_qfq *= qfq_factor;

            factors.push(FqFactorItem {
                date: date_key,
                close_before: p_close,
                qfq_factor,
                hfq_factor,
                div_per_share: parts.div_per_share,
                bonus_ratio: parts.bonus_ratio,
                rights_ratio: parts.rights_ratio,
                rights_price: parts.rights_price,
            });
        }
    }

    FqFactorResult {
        factors,
        cumulative_qfq,
        cumulative_hfq: if cumulative_qfq.abs() > 1e-10 {
            1.0 / cumulative_qfq
        } else {
            1.0
        },
    }
}

/// 复权类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FqType {
    None = 0,
    Qfq = 1, // 前复权
    Hfq = 2, // 后复权
}

/// 因子组成部分 (除权日价格无关的部分)
#[derive(Debug, Clone, Copy)]
struct FactorParts {
    div_per_share: f64,
    bonus_ratio: f64,
    rights_ratio: f64,
    rights_price: f64,
}

/// 计算实际前复权因子
///
/// factor = (P_close - D + P_rights × R_rights) / (P_close × (1 + R_bonus + R_rights))
fn calc_qfq_factor(close_before: f64, parts: &FactorParts) -> f64 {
    let denominator = close_before * (1.0 + parts.bonus_ratio + parts.rights_ratio);
    let numerator = close_before - parts.div_per_share + parts.rights_price * parts.rights_ratio;

    if denominator.abs() < 1e-10 || close_before.abs() < 1e-10 {
        return 1.0;
    }
    numerator / denominator
}

/// 将价格四舍五入到指定小数位数 (不影响 f64 内部精度, 仅消除浮点尾噪声)
#[inline]
fn round_price(p: f64, places: u32) -> f64 {
    let scale = 10_f64.powi(places as i32);
    (p * scale).round() / scale
}

/// 查找事件前一交易日的收盘价
///
/// 先在 `bars` (主数据) 中搜索, 取最后一个日期 < date_key 的 bar；
/// 若未找到, 再在 `context_bars` (更早的历史数据) 中搜索。
fn find_close_before_event(
    bars: &[SecurityBar],
    context_bars: &[SecurityBar],
    date_key: u32,
) -> Option<f64> {
    // bars 中找 (正向迭代, 取最后一个日期 < date_key 的)
    if let Some(bar) = bars
        .iter()
        .take_while(|b| b.year * 10000 + b.month * 100 + b.day < date_key)
        .last()
    {
        return Some(bar.close);
    }
    // context_bars 中找 (反向迭代, 找最后一个日期 < date_key 的)
    context_bars
        .iter()
        .rev()
        .find(|b| b.year * 10000 + b.month * 100 + b.day < date_key)
        .map(|b| b.close)
}

/// 对个股 K 线数据执行复权调整 (原地修改)
///
/// `bars`: 未复权 K 线数据 (按日期升序)，将被调整
/// `context_bars`: 额外的历史 K 线数据 (按日期升序)，仅用于因子计算，不会被修改
/// `xdxr_list`: 该股票的除权除息历史 (由 get_xdxr_info 获取)
/// `fq_type`: 复权类型
///
/// 精度: 内部全程 f64 计算, 最终输出时保留 3 位小数 (避免浮点尾噪声,
/// 不影响累乘精度)。成交量不做调整。
///
/// ## 历史上下文
///
/// 当 `context_bars` 非空时, 调整器先在其中搜索每个除权日的前收盘价,
/// 再在 `bars` 中搜索。这解决了早期除权事件位于请求 K 线数据范围之外
/// 导致因子被静默丢弃的问题 (例如 300750 的 800 根日 K 线仅覆盖
/// 2023-01 至 2026-05, 而 2019–2022 年有 4 个除权除息事件)。
pub fn adjust_security_bars(
    bars: &mut [SecurityBar],
    context_bars: &[SecurityBar],
    xdxr_list: &[XdXrInfo],
    fq_type: FqType,
) {
    if fq_type == FqType::None || bars.is_empty() || xdxr_list.is_empty() {
        return;
    }

    // 1. 构建除权事件 (升序)
    let mut events: Vec<(u32, FactorParts)> = Vec::new();
    for xd in xdxr_list {
        if xd.category != 1 {
            continue;
        }
        let date_key = xd.year * 10000 + xd.month * 100 + xd.day;
        events.push((
            date_key,
            FactorParts {
                div_per_share: xd.fenhong.unwrap_or(0.0) / 10.0,
                bonus_ratio: xd.songzhuangu.unwrap_or(0.0) / 10.0,
                rights_ratio: xd.peigu.unwrap_or(0.0) / 10.0,
                rights_price: xd.peigujia.unwrap_or(0.0),
            },
        ));
    }
    events.sort_by_key(|e| e.0);

    // 2. 构建因子查找表: date_key → 前复权因子
    //    从 context_bars → bars 中找到每个除权日的前收盘价
    let mut factor_map: BTreeMap<u32, f64> = BTreeMap::new();
    for &(date_key, parts) in &events {
        if let Some(p_close) = find_close_before_event(bars, context_bars, date_key) {
            let factor = calc_qfq_factor(p_close, &parts);
            factor_map.insert(date_key, factor);
        }
    }

    if factor_map.is_empty() {
        return;
    }

    match fq_type {
        FqType::Qfq => {
            let mut cumulative = 1.0;
            let mut event_iter = events.iter().rev().peekable();
            for bar in bars.iter_mut().rev() {
                let bar_key = bar.year * 10000 + bar.month * 100 + bar.day;
                while let Some(&(evt_date, _parts)) = event_iter.peek() {
                    if *evt_date > bar_key {
                        if let Some(&factor) = factor_map.get(evt_date) {
                            cumulative *= factor;
                        }
                        event_iter.next();
                    } else {
                        break;
                    }
                }
                if (cumulative - 1.0).abs() > 1e-10 {
                    bar.open *= cumulative;
                    bar.high *= cumulative;
                    bar.low *= cumulative;
                    bar.close *= cumulative;
                }
            }
        }
        FqType::Hfq => {
            let mut cumulative = 1.0;
            let mut event_iter = events.iter().peekable();
            for bar in bars.iter_mut() {
                let bar_key = bar.year * 10000 + bar.month * 100 + bar.day;
                while let Some(&(evt_date, _)) = event_iter.peek() {
                    if *evt_date <= bar_key {
                        if let Some(&factor) = factor_map.get(evt_date) {
                            cumulative *= 1.0 / factor;
                        }
                        event_iter.next();
                    } else {
                        break;
                    }
                }
                if (cumulative - 1.0).abs() > 1e-10 {
                    bar.open *= cumulative;
                    bar.high *= cumulative;
                    bar.low *= cumulative;
                    bar.close *= cumulative;
                }
            }
        }
        FqType::None => {}
    }

    // 4. 最终精度控制: 消除浮点尾噪声
    for bar in bars.iter_mut() {
        bar.open = round_price(bar.open, FQ_PRICE_PRECISION);
        bar.high = round_price(bar.high, FQ_PRICE_PRECISION);
        bar.low = round_price(bar.low, FQ_PRICE_PRECISION);
        bar.close = round_price(bar.close, FQ_PRICE_PRECISION);
    }
}

/// 对指数 K 线执行复权 (指数通常不需要复权, 保留接口一致性)
pub fn adjust_index_bars(bars: &mut [IndexBar], _xdxr_list: &[XdXrInfo], _fq_type: FqType) {
    // 指数不存在复权概念
    let _ = (bars, _xdxr_list, _fq_type);
}

#[cfg(test)]
#[path = "../../tests/internal/protocol_adjuster.rs"]
mod tests;
