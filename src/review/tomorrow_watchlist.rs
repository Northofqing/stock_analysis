//! v12 MVP-4 §7.6: 明日观察池 (R-07 推送).
//!
//! 设计: 4 类来源 (A档未触发/龙虎榜强票/涨停链龙头/可做T持仓) + 去重排序.

use chrono::Local;

/// 观察池来源枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchSource {
    AGradeNotTriggered,
    LhbStrong,
    LimitChainLeader,
    T0Candidate,
}

impl WatchSource {
    pub fn label(self) -> &'static str {
        match self {
            WatchSource::AGradeNotTriggered => "A档未触发",
            WatchSource::LhbStrong => "龙虎榜强票",
            WatchSource::LimitChainLeader => "涨停链龙头",
            WatchSource::T0Candidate => "可做T持仓",
        }
    }
}

/// 观察池条目
#[derive(Debug, Clone)]
pub struct WatchItem {
    pub code: String,
    pub name: String,
    pub topic: String,
    pub source: WatchSource,
    pub trigger: String,
    pub lo_price: f64,
    pub hi_price: f64,
    pub stop: f64,
    pub reason: String,
}

/// R-07 渲染
pub fn render_r07(items: &[WatchItem]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "📌 明日观察池（{}）\n",
        Local::now().format("%Y-%m-%d")
    ));
    for (i, it) in items.iter().enumerate() {
        s.push_str(&format!(
            "{}. {}({}) [{}] 来源: {}\n   触发{} | 低吸{:.2}~{:.2} | 止损{:.2}\n   理由: {}\n─────\n",
            i + 1,
            it.name,
            it.code,
            it.topic,
            it.source.label(),
            it.trigger,
            it.lo_price,
            it.hi_price,
            it.stop,
            it.reason,
        ));
    }
    s.push_str(&format!("共{}只 | 明日竞价后按 T-11 复核\n", items.len()));
    s
}

/// 去重 (按 code 留首个)
pub fn dedup(items: Vec<WatchItem>) -> Vec<WatchItem> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|i| seen.insert(i.code.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_item(code: &str, source: WatchSource) -> WatchItem {
        WatchItem {
            code: code.to_string(),
            name: format!("XX{}", code),
            topic: "AI硬件".into(),
            source,
            trigger: "距触发价 1.5%".into(),
            lo_price: 10.0,
            hi_price: 10.5,
            stop: 9.5,
            reason: "新闻+放量共振".into(),
        }
    }

    #[test]
    fn render_basic() {
        let items = vec![
            mock_item("600519", WatchSource::AGradeNotTriggered),
            mock_item("000001", WatchSource::LhbStrong),
        ];
        let s = render_r07(&items);
        assert!(s.contains("📌 明日观察池"));
        assert!(s.contains("600519"));
        assert!(s.contains("A档未触发"));
    }

    #[test]
    fn dedup_keeps_first() {
        let items = vec![
            mock_item("600519", WatchSource::AGradeNotTriggered),
            mock_item("600519", WatchSource::LhbStrong), // 重复
            mock_item("000001", WatchSource::LhbStrong),
        ];
        let deduped = dedup(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].source, WatchSource::AGradeNotTriggered);
    }

    #[test]
    fn empty_items_renders_count() {
        let s = render_r07(&[]);
        assert!(s.contains("共0只"));
    }
}
