//! v11-P0-5+ 候选筛选台 — Commit A: 候选模型 + 多源合并去重
//!
//! ## 红线 (P5 §一 钉死)
//! **候选筛选台,不是买入决策台**。信号 90% 未验证(breakout/空中加油/dual_score),只做"过滤+去重+证据分层",**不给"买入指令"**。
//! 唯一能标 🟢 强证据的是 布林+MACD (v11 factor_ic 认可的 B 方案);其余标 🟡 参考/⚪ 题材,让人自己判断。
//!
//! ## 红线 2 (P5 §十 明确)
//! **不合成"买入分"** —— sentiment 的死法。`Candidate` 里**没有**任何"综合分/买入分"字段,只有分档证据列表。

use std::collections::{HashMap, HashSet};

/// 候选来源: 5 个 P0-4 移交的买入侧推送
///
/// 与 P0-4 推送盘点对应 (grill Q2 修订):
/// - A10 选股推荐Top3 → `StockPick`
/// - B3 优选候选 → `OptimalClose`
/// - B6 放量·自选 → `VolumeWatchlist`
/// - B7 放量·实盘优选 → `VolumeRealTrade`
/// - C4 产业链扫描 → `IndustryChain`
/// - C6 news_monitor opp push 留 P0-6+ 实际改造 (本 commit 不接)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CandidateSource {
    StockPick,
    OptimalClose,
    VolumeWatchlist,
    VolumeRealTrade,
    IndustryChain,
}

impl CandidateSource {
    pub fn label(self) -> &'static str {
        match self {
            CandidateSource::StockPick => "选股",
            CandidateSource::OptimalClose => "优选",
            CandidateSource::VolumeWatchlist => "放量自选",
            CandidateSource::VolumeRealTrade => "放量实盘",
            CandidateSource::IndustryChain => "产业链",
        }
    }
}

/// 证据分档 (P5 §3.2 — 不合成假分, 分三档标注可靠性)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EvidenceTier {
    /// 🟢 强证据 (已验证): 布林+MACD 主升浪/抄底信号 (B 方案, v11 factor_ic 认可)
    Strong,
    /// 🟡 参考证据 (未验证): breakout 置信 / 空中加油 / 放量 / 资金净流入
    Reference,
    /// ⚪ 题材证据: 所属产业链 + 板块热度
    Theme,
}

impl EvidenceTier {
    pub fn emoji(self) -> &'static str {
        match self {
            EvidenceTier::Strong => "🟢",
            EvidenceTier::Reference => "🟡",
            EvidenceTier::Theme => "⚪",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            EvidenceTier::Strong => "强",
            EvidenceTier::Reference => "参考",
            EvidenceTier::Theme => "题材",
        }
    }
}

/// 一行候选 (P5 §3.1 去重合并结果)
///
/// 同一票多源 → 合并成一行 + `sources` 列出所有出现源 (P5 §3.1 红线: 出现越多 ≈ 越多路信号)
#[derive(Debug, Clone)]
pub struct CandidateEntry {
    pub code: String,
    pub name: String,
    pub sources: Vec<CandidateSource>,
    pub tier: EvidenceTier,
    pub evidence: Vec<String>,
    pub current_price: f64,
    pub change_pct: f64,
}

impl CandidateEntry {
    /// source 数量 (用于排序: 多源 > 单源)
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// source 列表显示文字 (P5 §五 输出形态: "来源: 选股+优选+产业链 (3 路指向)")
    pub fn sources_label(&self) -> String {
        if self.sources.is_empty() {
            return "无源".to_string();
        }
        self.sources
            .iter()
            .map(|s| s.label())
            .collect::<Vec<_>>()
            .join("+")
    }

    /// 强证据 (tier == Strong) 才有 "未验证" 警告 (P5 §五 红线)
    pub fn needs_warning(&self) -> bool {
        self.tier != EvidenceTier::Strong
    }
}

/// 多源合并去重 (P5 §3.1)
///
/// 同一 code 在多个源出现 → 合并成一行, `sources` 列出所有源.
/// `items` 是 raw 候选 (各源输出), `source_map` 把每条 raw 映射到它的源.
///
/// **红线** (P5 §一 + §十):
/// - 不给"建议买入"字样
/// - 不合成假分
/// - 来源越少越不靠谱 (1 路 vs 3 路信号)
pub fn merge_candidates(
    items: Vec<(CandidateSource, String, String)>, // (source, code, name)
) -> Vec<CandidateEntry> {
    let mut by_code: HashMap<String, CandidateEntry> = HashMap::new();
    for (source, code, name) in items {
        by_code
            .entry(code.clone())
            .or_insert_with(|| CandidateEntry {
                code: code.clone(),
                name: name.clone(),
                sources: Vec::new(),
                tier: EvidenceTier::Theme, // 兜底, Commit B 会改成 Strong/Reference
                evidence: Vec::new(),
                current_price: 0.0,
                change_pct: 0.0,
            })
            .sources
            .push(source);
    }
    // 源去重 (同源同票 push 两次,只记一次)
    let mut out: Vec<CandidateEntry> = by_code
        .into_iter()
        .map(|(_, mut e)| {
            let mut seen: HashSet<CandidateSource> = HashSet::new();
            e.sources.retain(|s| seen.insert(*s));
            e
        })
        .collect();
    // 排序: 多源优先 (P5 §3.3 硬规则: 强证据优先 > 参考证据强度 > 题材热度)
    // Commit A 只做"多源优先", tier 排序留给 Commit B
    out.sort_by(|a, b| {
        b.source_count()
            .cmp(&a.source_count())
            .then_with(|| a.code.cmp(&b.code))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(s: CandidateSource) -> Vec<CandidateSource> {
        vec![s]
    }

    /// 同一票多源 → 合并成一行
    #[test]
    fn merge_same_code_multiple_sources() {
        let items = vec![
            (CandidateSource::StockPick, "000001".to_string(), "测试A".to_string()),
            (CandidateSource::OptimalClose, "000001".to_string(), "测试A".to_string()),
            (CandidateSource::IndustryChain, "000001".to_string(), "测试A".to_string()),
        ];
        let result = merge_candidates(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].code, "000001");
        assert_eq!(result[0].source_count(), 3);
        assert_eq!(
            result[0].sources_label(),
            "选股+优选+产业链",
            "来源标签拼接顺序应按插入顺序"
        );
    }

    /// 单源单条
    #[test]
    fn merge_single_source_single_item() {
        let items = vec![(
            CandidateSource::StockPick,
            "000001".to_string(),
            "测试A".to_string(),
        )];
        let result = merge_candidates(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_count(), 1);
        assert_eq!(result[0].sources_label(), "选股");
    }

    /// 多票多源 → 各占一行
    #[test]
    fn merge_different_codes() {
        let items = vec![
            (CandidateSource::StockPick, "000001".to_string(), "A".to_string()),
            (CandidateSource::OptimalClose, "000002".to_string(), "B".to_string()),
            (CandidateSource::IndustryChain, "000003".to_string(), "C".to_string()),
        ];
        let result = merge_candidates(items);
        assert_eq!(result.len(), 3);
    }

    /// 同源同票重复 → 只记一次
    #[test]
    fn merge_dedup_same_source() {
        let items = vec![
            (CandidateSource::StockPick, "000001".to_string(), "测试".to_string()),
            (CandidateSource::StockPick, "000001".to_string(), "测试".to_string()),
            (CandidateSource::StockPick, "000001".to_string(), "测试".to_string()),
        ];
        let result = merge_candidates(items);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_count(), 1, "同源重复应去重");
    }

    /// 多源优先排序 (P5 §3.3 硬规则: 强证据优先)
    #[test]
    fn merge_sort_by_source_count() {
        let items = vec![
            (CandidateSource::StockPick, "000001".to_string(), "A".to_string()),
            (CandidateSource::OptimalClose, "000002".to_string(), "B".to_string()),
            (
                CandidateSource::StockPick,
                "000003".to_string(),
                "C".to_string(),
            ),
            (CandidateSource::IndustryChain, "000003".to_string(), "C".to_string()),
        ];
        let result = merge_candidates(items);
        assert_eq!(result[0].code, "000003", "2-source 票应排第一");
        assert_eq!(result[0].source_count(), 2);
    }
}
