//! kdj（从 indicators.rs 拆分）

use std::collections::VecDeque;

// ============================================================================
// KDJ
// ============================================================================

/// 单条 KDJ 数据点
#[derive(Debug, Clone, Default)]
pub struct KdjPoint {
    pub k: f64,
    pub d: f64,
    pub j: f64,
}

/// 计算 KDJ 序列
///
/// `highs`, `lows`, `closes` 按时间升序排列，长度必须一致。
/// 使用单调双端队列实现 O(n) 滑动窗口最大/最小值查询。
pub fn calc_kdj(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    n: usize,  // RSV 周期，默认 9
    m1: usize, // K 平滑周期，默认 3
    m2: usize, // D 平滑周期，默认 3
) -> Vec<KdjPoint> {
    let len = closes.len();
    if len == 0 {
        return Vec::new();
    }

    // 计算 RSV — 使用单调队列 O(n) 替代原来的 O(n·n) 滑动窗口扫描
    let mut rsv = vec![50.0_f64; len];
    // 递减队列：队首为窗口内最大值的 (index, value)
    let mut max_q: VecDeque<(usize, f64)> = VecDeque::new();
    // 递增队列：队首为窗口内最小值的 (index, value)
    let mut min_q: VecDeque<(usize, f64)> = VecDeque::new();

    for i in 0..len {
        // 移除窗口外元素
        while max_q.front().is_some_and(|&(idx, _)| idx + n <= i) {
            max_q.pop_front();
        }
        while min_q.front().is_some_and(|&(idx, _)| idx + n <= i) {
            min_q.pop_front();
        }

        // 维护递减性质：弹出队尾 ≤ 当前值的元素
        while max_q.back().is_some_and(|&(_, v)| v <= highs[i]) {
            max_q.pop_back();
        }
        max_q.push_back((i, highs[i]));

        // 维护递增性质：弹出队尾 ≥ 当前值的元素
        while min_q.back().is_some_and(|&(_, v)| v >= lows[i]) {
            min_q.pop_back();
        }
        min_q.push_back((i, lows[i]));

        let hh = max_q.front().map_or(highs[i], |&(_, v)| v);
        let ll = min_q.front().map_or(lows[i], |&(_, v)| v);

        if (hh - ll).abs() > 1e-10 {
            rsv[i] = (closes[i] - ll) / (hh - ll) * 100.0;
        }
    }

    // SMA 平滑
    let mut k_vals = vec![50.0_f64; len];
    let mut d_vals = vec![50.0_f64; len];
    let mut j_vals = vec![50.0_f64; len];

    for i in 0..len {
        if i == 0 {
            k_vals[i] = rsv[i];
        } else {
            k_vals[i] = k_vals[i - 1] * (m1 as f64 - 1.0) / m1 as f64 + rsv[i] / m1 as f64;
        }
    }

    for i in 0..len {
        if i == 0 {
            d_vals[i] = k_vals[i];
        } else {
            d_vals[i] = d_vals[i - 1] * (m2 as f64 - 1.0) / m2 as f64 + k_vals[i] / m2 as f64;
        }
    }

    for i in 0..len {
        j_vals[i] = 3.0 * k_vals[i] - 2.0 * d_vals[i];
    }

    (0..len)
        .map(|i| KdjPoint {
            k: k_vals[i],
            d: d_vals[i],
            j: j_vals[i],
        })
        .collect()
}
