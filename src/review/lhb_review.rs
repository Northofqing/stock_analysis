//! v12 MVP-4 §7.4: 龙虎榜复盘 (R-04 21:00 推送).
//!
//! 设计: 净买前五/买卖集中度/机构占比/主线一致性/次日风险.
//! 输出关键词黑名单 (禁席位人格化标签).

use chrono::Local;

/// 龙虎榜条目
#[derive(Debug, Clone)]
pub struct LhbTop5Item {
    pub code: String,
    pub name: String,
    pub net_buy_yi: f64,     // 净买 (亿元)
    pub reason: String,      // 上榜原因
    pub buy_seats_inst: u32, // 买方机构席位数
    pub buy_seats_inst_amt_wan: f64,
    pub buy_seats_other: u32,
    pub buy_seats_other_amt_wan: f64,
    pub buy_concentration_pct: f64, // 集中度
    pub sell_concentration_pct: f64,
    pub chain_match: bool,     // 主线一致
    pub next_day_risk: String, // 次日风险
}

/// 禁关键词黑名单 (personality label)
const BANNED_LABELS: &[&str] = &["游资大佬", "敢死队", "温州帮", "山东帮", "涨停敢死队"];

/// 检查输出是否含禁词, 返回 true = 含禁词
pub fn has_banned_label(text: &str) -> bool {
    BANNED_LABELS.iter().any(|b| text.contains(b))
}

/// R-04 渲染 (21:00 推)
pub fn render_r04(items: &[LhbTop5Item]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "🐉 龙虎榜净买前五（{} 21:00）\n",
        Local::now().format("%Y-%m-%d")
    ));
    for (i, it) in items.iter().enumerate() {
        s.push_str(&format!(
            "{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{:.0}万 其他{}席{:.0}万（集中度{:.0}%）\n   卖: 集中度{:.0}%\n   主线一致: {}\n   次日风险: {}\n─────\n",
            i + 1,
            it.name,
            it.code,
            it.net_buy_yi,
            it.reason,
            it.buy_seats_inst,
            it.buy_seats_inst_amt_wan,
            it.buy_seats_other,
            it.buy_seats_other_amt_wan,
            it.buy_concentration_pct,
            it.sell_concentration_pct,
            if it.chain_match { "是" } else { "否" },
            it.next_day_risk,
        ));
    }
    s.push_str("仅结构化事实, 不含席位风格推断\n");
    s
}

/// 校验 R-04 渲染含禁词
pub fn validate_r04(s: &str) -> bool {
    !has_banned_label(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_item() -> LhbTop5Item {
        LhbTop5Item {
            code: "600519".into(),
            name: "XX".into(),
            net_buy_yi: 2.5,
            reason: "涨幅偏离值达7%".into(),
            buy_seats_inst: 2,
            buy_seats_inst_amt_wan: 8000.0,
            buy_seats_other: 3,
            buy_seats_other_amt_wan: 5000.0,
            buy_concentration_pct: 60.0,
            sell_concentration_pct: 30.0,
            chain_match: true,
            next_day_risk: "高 (涨幅偏大, 谨防回调)".into(),
        }
    }

    #[test]
    fn render_basic() {
        let s = render_r04(&[mock_item()]);
        assert!(s.contains("🐉"));
        assert!(s.contains("净买2.5亿"));
        assert!(s.contains("主线一致: 是"));
    }

    #[test]
    fn no_banned_label_clean() {
        let s = render_r04(&[mock_item()]);
        assert!(validate_r04(&s));
    }

    #[test]
    fn banned_label_detected() {
        let mut it = mock_item();
        it.reason = "游资大佬入场".into();
        let s = render_r04(&[it]);
        assert!(!validate_r04(&s));
    }

    #[test]
    fn empty_items() {
        let s = render_r04(&[]);
        assert!(s.contains("🐉"));
        assert!(s.contains("仅结构化事实"));
    }
}
