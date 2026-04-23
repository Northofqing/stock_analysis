//! kdj（从 indicators.rs 拆分）


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
pub fn calc_kdj(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    n: usize,   // RSV 周期，默认 9
    m1: usize,  // K 平滑周期，默认 3
    m2: usize,  // D 平滑周期，默认 3
) -> Vec<KdjPoint> {
    let len = closes.len();
    if len == 0 {
        return Vec::new();
    }

    // 计算 RSV
    let mut rsv = vec![50.0_f64; len];
    for i in 0..len {
        let start = if i + 1 >= n { i + 1 - n } else { 0 };
        let hh = highs[start..=i]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[start..=i]
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        if (hh - ll).abs() > 1e-10 {
            rsv[i] = (closes[i] - ll) / (hh - ll) * 100.0;
        }
    }

    // SMA 平滑：K_i = K_{i-1} * (m-1)/m + RSV_i * 1/m
    let mut k_vals = vec![50.0_f64; len];
    let mut d_vals = vec![50.0_f64; len];
    let mut j_vals = vec![50.0_f64; len];

    for i in 0..len {
        if i == 0 {
            k_vals[i] = rsv[i];
        } else {
            k_vals[i] = k_vals[i - 1] * (m1 as f64 - 1.0) / m1 as f64
                + rsv[i] / m1 as f64;
        }
    }

    for i in 0..len {
        if i == 0 {
            d_vals[i] = k_vals[i];
        } else {
            d_vals[i] = d_vals[i - 1] * (m2 as f64 - 1.0) / m2 as f64
                + k_vals[i] / m2 as f64;
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

