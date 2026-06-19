//! 周度复盘 SOP — 每周五推送。

use chrono::{Datelike, Local};

/// 周五推送的周度复盘清单
pub fn weekly_sop(holdings_count: usize, exclusion_hits: usize, limit_violations: usize) -> String {
    let today = Local::now();
    let mut lines = vec![
        format!("📋 周度复盘 SOP（{}）", today.format("%Y-%m-%d")),
        "━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        String::new(),
    ];

    lines.push("☐ 1. 查污染源：持仓/自选有无命中排除清单？".to_string());
    if exclusion_hits > 0 {
        lines.push(format!("   ⚠️ 本周 {} 项命中，请复核", exclusion_hits));
    } else {
        lines.push("   ✅ 无污染源".to_string());
    }

    lines.push(format!("☐ 2. 验虹吸：{} 只持仓成交额是否仍居全市场前列？", holdings_count));
    lines.push("   → 萎缩为边缘跟风股则调仓到同板块领涨龙头".to_string());

    lines.push("☐ 3. 复核逻辑：最初的买入逻辑是否还在？".to_string());
    lines.push("   → 逻辑破坏即走，不等价格证明你错".to_string());

    lines.push("☐ 4. 调配现金：总趋势向上→现金可降至30%；市场风声鹤唳→锁死40%".to_string());

    lines.push(format!("☐ 5. 检验止损线：本周 {} 项超标，触线的有没有执行？", limit_violations));
    if limit_violations > 0 {
        lines.push(format!("   ⚠️ {} 项超标待处理", limit_violations));
    }

    lines.join("\n")
}

/// 判断今天是否周五
pub fn is_friday() -> bool {
    Local::now().weekday() == chrono::Weekday::Fri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sop_contains_all_items() {
        let text = weekly_sop(6, 0, 3);
        assert!(text.contains("查污染源"));
        assert!(text.contains("验虹吸"));
        assert!(text.contains("复核逻辑"));
        assert!(text.contains("调配现金"));
        assert!(text.contains("检验止损"));
        assert!(text.contains("3 项超标"));
    }
}
