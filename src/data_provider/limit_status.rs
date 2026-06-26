//! 涨跌停价 / ST / 停牌 状态计算器
//!
//! 单一来源：从股票代码 + 昨收 + 名称推出所有限价相关字段。
//! 不依赖 market_analyzer/limit_up.rs（避免循环依赖）。
//!
//! 修复：QUANT_ANALYST_REVIEW.md §1.1
//! 原 bug：data_provider 中 is_limit_up/down/suspended 硬编码 false，全系统涨跌停检测是死代码。

use crate::data_provider::KlineData;
use serde::{Deserialize, Serialize};

/// 涨跌停状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LimitStatus {
    /// 涨跌停幅度（如 0.10 表示 ±10%）
    pub limit_pct: f64,
    /// 涨停价（昨收 * (1 + limit_pct)，四舍五入到 0.01）
    pub limit_up_price: f64,
    /// 跌停价（昨收 * (1 - limit_pct)，四舍五入到 0.01）
    pub limit_down_price: f64,
    pub is_st: bool,
    /// 停牌状态：默认 false；查询 sse/szse 公告后填
    pub is_suspended: bool,
}

/// 涨跌停价计算器
pub struct LimitStatusCalculator;

impl LimitStatusCalculator {
    pub fn new() -> Self {
        Self
    }

    /// 计算涨跌停价。
    /// `prev_close` 必须 > 0，否则 limit_up_price/limit_down_price 为 0
    /// （异常输入由调用方 warn 后跳过）。
    pub fn calculate(&self, code: &str, prev_close: f64, name: &str) -> LimitStatus {
        let is_st = is_st_stock(name);
        let board = detect_board(code);
        let limit_pct = match (board, is_st) {
            (_, true) => 0.05,
            (Board::Main, _) => 0.10,
            (Board::ChiNext, _) => 0.20,
            (Board::Star, _) => 0.20,
            (Board::Bj, _) => 0.30,
        };
        let (limit_up_price, limit_down_price) = if prev_close > 0.0 {
            (
                round_to_cent(prev_close * (1.0 + limit_pct)),
                round_to_cent(prev_close * (1.0 - limit_pct)),
            )
        } else {
            (0.0, 0.0)
        };
        LimitStatus {
            limit_pct,
            limit_up_price,
            limit_down_price,
            is_st,
            is_suspended: false,
        }
    }
}

/// 把 LimitStatusCalculator 的结果填到 KlineData。
/// `code` 是股票代码（KlineData 本身不带 code，由调用方传入）。
/// `prev_close` 是昨日收盘。
/// `name` 用于 ST 判定。
pub fn fill_limit_flags(
    calc: &LimitStatusCalculator,
    code: &str,
    k: &mut KlineData,
    prev_close: f64,
    name: &str,
) {
    let s = calc.calculate(code, prev_close, name);
    // 容忍 ±0.5 分的浮点误差
    k.is_limit_up = s.limit_up_price > 0.0 && (k.close - s.limit_up_price).abs() < 0.005;
    k.is_limit_down = s.limit_down_price > 0.0 && (k.close - s.limit_down_price).abs() < 0.005;
    k.is_suspended = s.is_suspended; // 默认 false，查询 sse 后填
}

/// 批量为 K 线列表（按日期降序）填涨跌停标记。
///
/// - `klines[0]`（最新一根，即"今日"）的 prev_close 用 `klines[1].close`（昨日）
/// - `klines[i]`（i>=1）的 prev_close 用 `klines[i+1].close`
///
/// 容忍停牌 / 节假日造成的 K 线间隔：前一根 bar 的 close 仍是当时最近一笔
/// 可得收盘价，作为今日的"昨日收盘"是合理的近似。
///
/// `name` 可为 None（非 ST 假设）；如有 name 应调用方提供。
pub fn apply_limit_flags_inplace(
    code: &str,
    name: Option<&str>,
    klines: &mut [KlineData],
) {
    if klines.is_empty() {
        return;
    }
    let calc = LimitStatusCalculator::new();
    let name_str = name.unwrap_or("");
    let n = klines.len();
    // 倒序遍历（时间从旧到新），保持 prev_close 来源简单
    // 由于 klines 已按日期降序，倒序遍历的下一个就是时间上更早的一根
    for i in (0..n).rev() {
        let prev_close = if i + 1 < n {
            klines[i + 1].close
        } else {
            // 最旧一根没有 prev_close，留 0，fill_limit_flags 会跳过
            0.0
        };
        fill_limit_flags(&calc, code, &mut klines[i], prev_close, name_str);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Board {
    Main,
    ChiNext,
    Star,
    Bj,
}

fn detect_board(code: &str) -> Board {
    if code.starts_with("688") {
        Board::Star
    } else if code.starts_with("300") || code.starts_with("301") {
        Board::ChiNext
    } else if code.starts_with("8") || code.starts_with("4") || code.starts_with("920") {
        Board::Bj
    } else {
        Board::Main
    }
}

/// 严格匹配：^(ST|*ST|S*ST|SST)
fn is_st_stock(name: &str) -> bool {
    name.starts_with("*ST")
        || name.starts_with("S*ST")
        || name.starts_with("SST")
        || name.starts_with("ST")
}

/// 四舍五入到 0.01（A 股价格最小变动单位）
fn round_to_cent(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl Default for LimitStatusCalculator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn kline(_code: &str, close: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 6, 27).unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000_000.0,
            amount: 10_000_000.0,
            pct_chg: 0.0,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
        }
    }

    #[test]
    fn detect_main() {
        assert_eq!(detect_board("600000"), Board::Main);
        assert_eq!(detect_board("000001"), Board::Main);
    }

    #[test]
    fn detect_chinext() {
        assert_eq!(detect_board("300750"), Board::ChiNext);
        assert_eq!(detect_board("301000"), Board::ChiNext);
    }

    #[test]
    fn detect_star() {
        assert_eq!(detect_board("688981"), Board::Star);
    }

    #[test]
    fn detect_bj() {
        assert_eq!(detect_board("830799"), Board::Bj);
        assert_eq!(detect_board("400001"), Board::Bj);
        assert_eq!(detect_board("920001"), Board::Bj);
    }

    #[test]
    fn st_match_strict() {
        assert!(is_st_stock("*ST华微"));
        assert!(is_st_stock("ST康美"));
        assert!(is_st_stock("SST集成"));
        assert!(is_st_stock("S*ST海伦"));
        // 不应误判
        assert!(!is_st_stock("浦发银行"));
        assert!(!is_st_stock("华微电子"));
        assert!(!is_st_stock("宁波银行"));
    }

    #[test]
    fn rounding() {
        assert!((round_to_cent(11.055) - 11.06).abs() < 1e-6);
        assert!((round_to_cent(9.005) - 9.01).abs() < 1e-6);
        assert!((round_to_cent(11.054) - 11.05).abs() < 1e-6);
    }

    #[test]
    fn main_board_10pct() {
        let calc = LimitStatusCalculator::new();
        let s = calc.calculate("600000", 10.0, "浦发银行");
        assert!((s.limit_pct - 0.10).abs() < 1e-6);
        assert!((s.limit_up_price - 11.0).abs() < 1e-6);
        assert!((s.limit_down_price - 9.0).abs() < 1e-6);
        assert!(!s.is_st);
    }

    #[test]
    fn chinext_20pct() {
        let calc = LimitStatusCalculator::new();
        let s = calc.calculate("300750", 100.0, "宁德时代");
        assert!((s.limit_pct - 0.20).abs() < 1e-6);
        assert!((s.limit_up_price - 120.0).abs() < 1e-6);
        assert!((s.limit_down_price - 80.0).abs() < 1e-6);
    }

    #[test]
    fn star_20pct() {
        let calc = LimitStatusCalculator::new();
        let s = calc.calculate("688981", 50.0, "中芯国际");
        assert!((s.limit_pct - 0.20).abs() < 1e-6);
        assert!((s.limit_up_price - 60.0).abs() < 1e-6);
        assert!((s.limit_down_price - 40.0).abs() < 1e-6);
    }

    #[test]
    fn bj_30pct() {
        let calc = LimitStatusCalculator::new();
        let s = calc.calculate("830799", 20.0, "艾融软件");
        assert!((s.limit_pct - 0.30).abs() < 1e-6);
        assert!((s.limit_up_price - 26.0).abs() < 1e-6);
        assert!((s.limit_down_price - 14.0).abs() < 1e-6);
    }

    #[test]
    fn st_5pct_overrides_board() {
        let calc = LimitStatusCalculator::new();
        // 创业板 ST 仍是 5%
        let s = calc.calculate("300001", 10.0, "*ST华微");
        assert!((s.limit_pct - 0.05).abs() < 1e-6);
        assert!(s.is_st);
    }

    #[test]
    fn zero_prev_close_returns_zero_limit() {
        let calc = LimitStatusCalculator::new();
        let s = calc.calculate("600000", 0.0, "X");
        assert_eq!(s.limit_up_price, 0.0);
        assert_eq!(s.limit_down_price, 0.0);
    }

    #[test]
    fn fill_limit_flags_hits_limit_up() {
        let calc = LimitStatusCalculator::new();
        let mut k = kline("600000", 11.0);
        fill_limit_flags(&calc, "600000", &mut k, 10.0, "浦发银行");
        assert!(k.is_limit_up);
        assert!(!k.is_limit_down);
    }

    #[test]
    fn fill_limit_flags_hits_limit_down() {
        let calc = LimitStatusCalculator::new();
        let mut k = kline("600000", 9.0);
        fill_limit_flags(&calc, "600000", &mut k, 10.0, "浦发银行");
        assert!(k.is_limit_down);
        assert!(!k.is_limit_up);
    }

    #[test]
    fn fill_limit_flags_no_flag_in_range() {
        let calc = LimitStatusCalculator::new();
        let mut k = kline("600000", 10.5);
        fill_limit_flags(&calc, "600000", &mut k, 10.0, "浦发银行");
        assert!(!k.is_limit_up);
        assert!(!k.is_limit_down);
    }

    #[test]
    fn fill_limit_flags_st_5pct() {
        let calc = LimitStatusCalculator::new();
        let mut k = kline("000001", 10.5); // close=10.5 = 5% 涨停
        fill_limit_flags(&calc, "000001", &mut k, 10.0, "*ST华微");
        assert!(k.is_limit_up);
    }

    #[test]
    fn apply_limit_flags_to_desc_list() {
        // 按日期降序: 11.0(今天), 10.0(昨天), 9.5(前天)
        // 今天是涨停 (11.0 == 10*1.1)
        // 昨天 close=10.0, prev=9.5, limit_down=9.5*0.9=8.55, 在范围内, 无标记
        // 前天 close=9.5, 没有 prev, 不填
        let mut klines = vec![
            { let mut k = kline("600000", 11.0); k.date = NaiveDate::from_ymd_opt(2026, 6, 27).unwrap(); k },
            { let mut k = kline("600000", 10.0); k.date = NaiveDate::from_ymd_opt(2026, 6, 26).unwrap(); k },
            { let mut k = kline("600000", 9.5);  k.date = NaiveDate::from_ymd_opt(2026, 6, 25).unwrap(); k },
        ];
        apply_limit_flags_inplace("600000", Some("浦发银行"), &mut klines);
        assert!(klines[0].is_limit_up, "今天 close=11.0 = 10*1.1, 应是涨停");
        assert!(!klines[0].is_limit_down);
        assert!(!klines[1].is_limit_up, "昨天在范围内");
        assert!(!klines[1].is_limit_down);
        // 前天没有 prev_close, 不填
        assert!(!klines[2].is_limit_up);
        assert!(!klines[2].is_limit_down);
    }

    #[test]
    fn apply_limit_flags_to_empty() {
        let mut klines: Vec<KlineData> = vec![];
        apply_limit_flags_inplace("600000", None, &mut klines);
        // 不应 panic
    }
}
