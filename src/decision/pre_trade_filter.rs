//! v12 PR2-2.3: 交易前排雷清单 (pre_trade_filter).
//!
//! 设计: 在 candidate 触发买入 / 持仓操作前过滤, 返回过滤原因.
//!       与 `decision::exclusion` 不同 — exclusion 排除板块, pre_trade_filter 排除**个股风险**.
//!
//! 规则 (BR-022 衍生 + v12 §13):
//!   1. 停牌 → 拒绝 (is_suspended)
//!   2. 涨停一字板 → 拒绝 (不能买入, 走 PR3 paper_trade)
//!   3. 解禁 N 日内 (默认 5 个交易日) → 标注风险, 不直接拒绝 (走公告关键词兜底)
//!   4. 业绩雷 (同比已亏 / 公告预减) → 拒绝
//!   5. 质押比例 → **暂缺数据源**, 标注 "暂缺数据源" 不假装生效 (PR2-2.3 显式要求)
//!   6. 减持到期 → 公告关键词兜底标注
//!
//! 策略: 数据缺失一律走"标注而非拒绝", 避免误杀 (AGENTS §2.2).

use std::collections::HashSet;

/// 排雷结果: 是否通过 + 拒绝/标注原因
#[derive(Clone, Debug)]
pub struct FilterResult {
    pub pass: bool,
    pub reasons: Vec<String>,
    /// 仅标注, 不影响 pass=true (例如解禁风险, 质押数据缺失)
    pub warnings: Vec<String>,
}

impl FilterResult {
    pub fn pass() -> Self {
        Self {
            pass: true,
            reasons: vec![],
            warnings: vec![],
        }
    }

    pub fn reject(reason: impl Into<String>) -> Self {
        Self {
            pass: false,
            reasons: vec![reason.into()],
            warnings: vec![],
        }
    }

    pub fn with_warning(mut self, w: impl Into<String>) -> Self {
        self.warnings.push(w.into());
        self
    }
}

/// 入参: 排雷所需的个股快照
///
/// 字段尽量 Optional — 缺失即按"保守标注"处理, 不静默填充.
#[derive(Clone, Debug, Default)]
pub struct StockFilterInput {
    pub code: String,
    pub name: String,
    /// 停牌 (None = 数据缺失, 假设未停牌)
    pub is_suspended: Option<bool>,
    /// 涨停一字板 (None = 数据缺失)
    pub is_limit_up_locked: Option<bool>,
    /// 解禁日期 (None = 数据缺失)
    pub unlock_date: Option<String>,
    /// 业绩同比 (None = 数据缺失). true = 同比已亏
    pub yoy_loss: Option<bool>,
    /// 公告预减 (None = 数据缺失)
    pub announced_pre_cut: Option<bool>,
    /// 公告减持到期 (None = 数据缺失)
    pub announced_unlock_to_sell: Option<bool>,
    /// 质押比例 (None = 数据缺失, BR-022 显式标注)
    pub pledge_ratio_pct: Option<f64>,
}

/// PR2-2.3 主评估函数
pub fn evaluate(input: &StockFilterInput, now_date: &str) -> FilterResult {
    let mut result = FilterResult::pass();

    // 1. 停牌 → 拒绝
    if matches!(input.is_suspended, Some(true)) {
        return FilterResult::reject(format!("停牌 (code={}) — 不可交易", input.code));
    }

    // 2. 涨停一字板 → 拒绝 (不可买入)
    if matches!(input.is_limit_up_locked, Some(true)) {
        return FilterResult::reject(format!(
            "涨停一字板 (code={}) — 不可买入, 走 PR3 paper_trade",
            input.code
        ));
    }

    // 3. 业绩雷 (同比已亏 / 公告预减) → 拒绝
    if matches!(input.yoy_loss, Some(true)) {
        return FilterResult::reject(format!("业绩雷: 同比已亏 (code={})", input.code));
    }
    if matches!(input.announced_pre_cut, Some(true)) {
        return FilterResult::reject(format!("业绩雷: 公告预减 (code={})", input.code));
    }

    // 4. 解禁 N 日内 → 标注风险
    //    `days = now - unlock`, 正值 = 解禁已过去 (近期解禁), 负值 = 解禁未来 (即将解禁)
    //    |days| ≤ 5 视为解禁窗口期内 (前后 5 个日历日)
    if let Some(unlock_date) = &input.unlock_date {
        if let Some(days) = days_between(now_date, unlock_date) {
            if days.abs() <= 5 {
                let label = if days >= 0 {
                    format!("解禁已 {} 日", days)
                } else {
                    format!("解禁将至 (-{} 日)", days)
                };
                result = result.with_warning(format!(
                    "解禁窗口期内 ({} {}, {})",
                    label, unlock_date, "BR-022"
                ));
            }
        } else {
            result = result.with_warning(format!(
                "解禁日解析失败: {} (公告关键词兜底标注)",
                unlock_date
            ));
        }
    }

    // 5. 减持到期 → 公告关键词兜底标注
    if matches!(input.announced_unlock_to_sell, Some(true)) {
        result = result.with_warning("公告减持到期 (公告关键词兜底)".to_string());
    }

    // 6. 质押比例 → 暂缺数据源, 标注不假装
    // BR-022 显式要求: "质押条目显式标注'暂缺数据源', 不假装生效"
    if input.pledge_ratio_pct.is_none() {
        result = result.with_warning("质押比例: 暂缺数据源 (BR-022)".to_string());
    } else if let Some(r) = input.pledge_ratio_pct {
        if r > 50.0 {
            result = result.with_warning(format!("高质押: {:.1}% (阈值 50%)", r));
        }
    }

    result
}

/// 工具: 两个日期字符串 (YYYY-MM-DD) 相距天数 (now_date - unlock_date).
///
/// 返回 None 表示解析失败.
fn days_between(now_date: &str, unlock_date: &str) -> Option<i64> {
    use chrono::NaiveDate;
    let now = NaiveDate::parse_from_str(now_date, "%Y-%m-%d").ok()?;
    let unlock = NaiveDate::parse_from_str(unlock_date, "%Y-%m-%d").ok()?;
    Some((now - unlock).num_days())
}

/// PR2-2.3 批量过滤: 对一个候选列表应用 evaluate, 返回通过 + 拒绝明细.
pub fn filter_batch(inputs: &[StockFilterInput], now_date: &str) -> FilterBatch {
    let mut pass = Vec::new();
    let mut reject = Vec::new();
    let mut warned: HashSet<String> = HashSet::new();

    for inp in inputs {
        let r = evaluate(inp, now_date);
        if !r.warnings.is_empty() {
            for w in &r.warnings {
                warned.insert(w.clone());
            }
        }
        if r.pass {
            pass.push(inp.code.clone());
        } else {
            reject.push((inp.code.clone(), r.reasons.join("; ")));
        }
    }
    FilterBatch {
        pass,
        reject,
        warnings: warned.into_iter().collect(),
    }
}

#[derive(Debug, Default)]
pub struct FilterBatch {
    pub pass: Vec<String>,
    pub reject: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(code: &str) -> StockFilterInput {
        StockFilterInput {
            code: code.to_string(),
            name: format!("测试{}", code),
            ..Default::default()
        }
    }

    // ---- 通过场景 ----

    #[test]
    fn pass_when_all_safe() {
        let r = evaluate(&base("600000"), "2026-07-05");
        assert!(r.pass);
        assert!(r.reasons.is_empty());
    }

    #[test]
    fn pass_when_data_all_missing() {
        // 数据全 None → 保守标注, 不拒绝
        let r = evaluate(&base("600000"), "2026-07-05");
        assert!(r.pass);
        // 应有质押 + 公告关键词等"暂缺数据源"标注
        assert!(r.warnings.iter().any(|w| w.contains("质押")));
    }

    // ---- 拒绝场景 ----

    #[test]
    fn reject_when_suspended() {
        let mut inp = base("600000");
        inp.is_suspended = Some(true);
        let r = evaluate(&inp, "2026-07-05");
        assert!(!r.pass);
        assert!(r.reasons[0].contains("停牌"));
    }

    #[test]
    fn reject_when_limit_up_locked() {
        let mut inp = base("600000");
        inp.is_limit_up_locked = Some(true);
        let r = evaluate(&inp, "2026-07-05");
        assert!(!r.pass);
        assert!(r.reasons[0].contains("涨停一字板"));
    }

    #[test]
    fn reject_when_yoy_loss() {
        let mut inp = base("600000");
        inp.yoy_loss = Some(true);
        let r = evaluate(&inp, "2026-07-05");
        assert!(!r.pass);
        assert!(r.reasons[0].contains("业绩雷"));
    }

    #[test]
    fn reject_when_pre_cut() {
        let mut inp = base("600000");
        inp.announced_pre_cut = Some(true);
        let r = evaluate(&inp, "2026-07-05");
        assert!(!r.pass);
        assert!(r.reasons[0].contains("预减"));
    }

    // ---- 标注场景 ----

    #[test]
    fn warn_when_unlock_within_5_days_future() {
        // 解禁在未来 3 日 (2026-07-05 → 2026-07-08), days=-3
        let mut inp = base("600000");
        inp.unlock_date = Some("2026-07-08".to_string());
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(
            r.warnings.iter().any(|w| w.contains("解禁")),
            "未来 3 日解禁应标注"
        );
    }

    #[test]
    fn warn_when_unlock_today() {
        let mut inp = base("600000");
        inp.unlock_date = Some("2026-07-05".to_string());
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(r.warnings.iter().any(|w| w.contains("解禁已 0")));
    }

    #[test]
    fn warn_when_unlock_past_within_5_days() {
        // 解禁已过 2 日 (2026-07-05 → 2026-07-03), days=2
        let mut inp = base("600000");
        inp.unlock_date = Some("2026-07-03".to_string());
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(r.warnings.iter().any(|w| w.contains("解禁已 2")));
    }

    #[test]
    fn no_warn_when_unlock_far() {
        // 解禁在未来 30 日
        let mut inp = base("600000");
        inp.unlock_date = Some("2026-08-15".to_string());
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(!r.warnings.iter().any(|w| w.contains("解禁")));
    }

    #[test]
    fn warn_when_unlock_date_parse_fail() {
        let mut inp = base("600000");
        inp.unlock_date = Some("not-a-date".to_string());
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(r.warnings.iter().any(|w| w.contains("解析失败")));
    }

    #[test]
    fn warn_when_announced_unlock_to_sell() {
        let mut inp = base("600000");
        inp.announced_unlock_to_sell = Some(true);
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(r.warnings.iter().any(|w| w.contains("减持到期")));
    }

    // ---- 质押 (PR2-2.3 关键) ----

    #[test]
    fn pledge_missing_marks_warning_no_reject() {
        // None = 数据缺失, 仅标注
        let inp = base("600000");
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass, "质押数据缺失不应拒绝");
        assert!(r.warnings.iter().any(|w| w.contains("暂缺数据源")));
    }

    #[test]
    fn pledge_low_no_warning() {
        let mut inp = base("600000");
        inp.pledge_ratio_pct = Some(30.0);
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(!r.warnings.iter().any(|w| w.contains("质押")));
    }

    #[test]
    fn pledge_high_warns() {
        let mut inp = base("600000");
        inp.pledge_ratio_pct = Some(60.0);
        let r = evaluate(&inp, "2026-07-05");
        assert!(r.pass);
        assert!(r.warnings.iter().any(|w| w.contains("高质押")));
    }

    // ---- 批量 ----

    #[test]
    fn filter_batch_separates_pass_and_reject() {
        let mut a = base("600000"); // pass
        let mut b = base("000001");
        b.is_suspended = Some(true); // reject
        let mut c = base("688001");
        c.yoy_loss = Some(true); // reject
        let batch = filter_batch(&[a, b, c], "2026-07-05");
        assert_eq!(batch.pass, vec!["600000"]);
        assert_eq!(batch.reject.len(), 2);
    }

    #[test]
    fn days_between_basic() {
        assert_eq!(days_between("2026-07-10", "2026-07-05"), Some(5));
        assert_eq!(days_between("2026-07-05", "2026-07-05"), Some(0));
        assert_eq!(days_between("2026-07-01", "2026-07-05"), Some(-4));
        assert_eq!(days_between("2026-07-05", "invalid"), None);
    }
}
