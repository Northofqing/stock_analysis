//! v12 MVP-4 §7.8: T-11 竞价异动 (复用现有 AuctionVolume, 加横幅).
//!
//! 设计: 09:20-09:25 竞价阶段抓 TopN 量能异动 + 判读 (强承接/分歧/核按钮).

use chrono::Local;

/// 竞价异动条目
#[derive(Debug, Clone)]
pub struct AuctionAnomaly {
    pub code: String,
    pub name: String,
    pub gap_pct: f64,        // 高开幅度 (%)
    pub vol_ratio: f64,       // 量比
    pub tag: AuctionTag,      // 昨日涨停/观察池
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuctionTag {
    YesterdayLimitUp,
    WatchPool,
}

impl AuctionTag {
    pub fn label(self) -> &'static str {
        match self {
            AuctionTag::YesterdayLimitUp => "昨日涨停",
            AuctionTag::WatchPool => "观察池",
        }
    }
}

/// 竞价情绪判读
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuctionSentiment {
    Strong,    // 强承接
    Divergence, // 分歧
    Nuke,      // 核按钮 (高开低走)
}

impl AuctionSentiment {
    pub fn label(self) -> &'static str {
        match self {
            AuctionSentiment::Strong => "强承接",
            AuctionSentiment::Divergence => "分歧",
            AuctionSentiment::Nuke => "核按钮",
        }
    }
}

/// T-11 渲染
pub fn render_t11(anomalies: &[AuctionAnomaly], sentiment: AuctionSentiment, watch_operable: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!("🌅 竞价异动 Top{}（{}）\n", anomalies.len(), Local::now().format("%H:%M")));
    for a in anomalies {
        s.push_str(&format!(
            "  {}({}) 高开{:+.1}% 量比{:.1} [{}]\n",
            a.name, a.code, a.gap_pct, a.vol_ratio, a.tag.label(),
        ));
    }
    s.push_str(&format!(
        "情绪判读: {}, 观察池今日{}\n",
        sentiment.label(),
        if watch_operable { "可操作" } else { "谨慎" },
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_basic() {
        let anomalies = vec![
            AuctionAnomaly { code: "600519".into(), name: "XX".into(), gap_pct: 2.5, vol_ratio: 3.2, tag: AuctionTag::YesterdayLimitUp },
            AuctionAnomaly { code: "000001".into(), name: "YY".into(), gap_pct: 5.1, vol_ratio: 8.0, tag: AuctionTag::WatchPool },
        ];
        let s = render_t11(&anomalies, AuctionSentiment::Strong, true);
        assert!(s.contains("🌅 竞价异动 Top2"));
        assert!(s.contains("强承接"));
        assert!(s.contains("可操作"));
    }

    #[test]
    fn render_nuke_sentiment() {
        let s = render_t11(&[], AuctionSentiment::Nuke, false);
        assert!(s.contains("核按钮"));
        assert!(s.contains("谨慎"));
    }

    #[test]
    fn empty_anomalies() {
        let s = render_t11(&[], AuctionSentiment::Divergence, false);
        assert!(s.contains("Top0"));
    }
}