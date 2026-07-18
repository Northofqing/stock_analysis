//! SKDJ（慢速 KDJ，EMA 平滑版）
//!
//! 与标准 KDJ 的区别：K、D 使用 EMA 平滑，对短期噪声更不敏感。
//! 项目默认参数对齐东方财富客户端：N=40, M=5。
//!
//! 公式：
//! - RSV = (close − LLV(low, N)) / (HHV(high, N) − LLV(low, N)) × 100
//! - K   = EMA(RSV, M)
//! - D   = EMA(K,   M)
//! - J   = 3K − 2D

use std::collections::VecDeque;

/// 单条 SKDJ 数据点
#[derive(Debug, Clone, Default)]
pub struct SkdjPoint {
    pub k: f64,
    pub d: f64,
    pub j: f64,
}

/// 计算 SKDJ 序列
///
/// `highs`, `lows`, `closes` 按时间升序排列，长度必须一致。
/// 使用单调双端队列实现 O(n) 滑动窗口最大/最小值查询。
///
/// - `n`: RSV 周期（默认 40）
/// - `m`: K/D EMA 平滑周期（默认 5）
pub fn calc_skdj(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    n: usize,
    m: usize,
) -> Vec<SkdjPoint> {
    let len = closes.len();
    if len == 0 || n == 0 || m == 0 {
        return Vec::new();
    }
    debug_assert_eq!(highs.len(), len);
    debug_assert_eq!(lows.len(), len);

    // ---- RSV — 单调队列 O(n) ----
    let mut rsv = vec![50.0_f64; len];
    let mut max_q: VecDeque<(usize, f64)> = VecDeque::new();
    let mut min_q: VecDeque<(usize, f64)> = VecDeque::new();

    for i in 0..len {
        while max_q.front().is_some_and(|&(idx, _)| idx + n <= i) {
            max_q.pop_front();
        }
        while min_q.front().is_some_and(|&(idx, _)| idx + n <= i) {
            min_q.pop_front();
        }

        while max_q.back().is_some_and(|&(_, v)| v <= highs[i]) {
            max_q.pop_back();
        }
        max_q.push_back((i, highs[i]));

        while min_q.back().is_some_and(|&(_, v)| v >= lows[i]) {
            min_q.pop_back();
        }
        min_q.push_back((i, lows[i]));

        let hh = max_q.front().map_or(highs[i], |&(_, v)| v);
        let ll = min_q.front().map_or(lows[i], |&(_, v)| v);
        let denom = hh - ll;
        rsv[i] = if denom.abs() < 1e-12 {
            50.0
        } else {
            (closes[i] - ll) / denom * 100.0
        };
    }

    // ---- EMA(M) on RSV → K, EMA(M) on K → D ----
    let alpha = 2.0 / (m as f64 + 1.0);

    let mut out = Vec::with_capacity(len);
    let mut k_prev = rsv[0];
    let mut d_prev = k_prev;
    for (i, &r) in rsv.iter().enumerate() {
        let k = if i == 0 {
            r
        } else {
            alpha * r + (1.0 - alpha) * k_prev
        };
        let d = if i == 0 {
            k
        } else {
            alpha * k + (1.0 - alpha) * d_prev
        };
        let j = 3.0 * k - 2.0 * d;
        out.push(SkdjPoint { k, d, j });
        k_prev = k;
        d_prev = d;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skdj_returns_same_length() {
        let highs = vec![10.0, 11.0, 12.0, 11.5, 12.5, 13.0, 12.8, 13.2];
        let lows = vec![9.0, 9.5, 10.0, 10.5, 11.0, 11.5, 12.0, 12.3];
        let closes = vec![9.5, 10.5, 11.5, 11.0, 12.0, 12.5, 12.4, 13.0];
        let out = calc_skdj(&highs, &lows, &closes, 5, 3);
        assert_eq!(out.len(), closes.len());
    }

    #[test]
    fn skdj_k_in_range() {
        let highs = vec![10.0, 11.0, 12.0, 11.5, 12.5, 13.0, 12.8, 13.2, 13.5, 14.0];
        let lows = vec![9.0, 9.5, 10.0, 10.5, 11.0, 11.5, 12.0, 12.3, 12.5, 13.0];
        let closes = vec![9.5, 10.5, 11.5, 11.0, 12.0, 12.5, 12.4, 13.0, 13.2, 13.8];
        let out = calc_skdj(&highs, &lows, &closes, 5, 3);
        for p in &out {
            assert!(p.k >= 0.0 && p.k <= 100.0, "K out of range: {}", p.k);
            assert!(p.d >= 0.0 && p.d <= 100.0, "D out of range: {}", p.d);
        }
    }

    #[test]
    fn skdj_empty_input() {
        let out = calc_skdj(&[], &[], &[], 40, 5);
        assert!(out.is_empty());
    }
}
