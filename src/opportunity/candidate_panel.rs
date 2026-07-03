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

// ============================================================================
// v11-P0-5+ Commit B: 证据分层 + 硬门槛过滤
// ============================================================================

/// 证据分档 (P5 §3.2 红线: 只有布林+MACD 能进 Strong)
///
/// 输入 evidence 文本列表 (LLM 或 breakout 输出), 命中关键词决定 tier:
/// - "布林+MACD" / "B方案" / "主升浪" / "抄底" → Strong (P5 §3.2 唯一能进强证据)
/// - 其它 (breakout / 空中加油 / 放量 / 资金净流入) → Reference (未验证)
/// - 仅产业链 (无技术/资金信号) → Theme
///
/// **红线** (P5 §一): breakout 置信 75 / 空中加油形态+8 等**未验证打分不能进 Strong**。
/// 即使 LLM 给 breakout 置信 99, 没 v11 factor_ic 验证, 一律 Reference.
pub fn classify_tier(evidence: &[String]) -> EvidenceTier {
    let combined = evidence.join(" ");
    // 唯一 Strong: 布林+MACD (P5 §3.2 唯一能进强证据, 强证据的"已验证"标签来自 v11 factor_ic 认可 B 方案)
    let strong_keywords = ["布林+MACD", "B方案", "主升浪启动", "B方案(已验证)", "布林+MACD主升浪"];
    if strong_keywords.iter().any(|kw| combined.contains(kw)) {
        return EvidenceTier::Strong;
    }
    // Reference: breakout / 空中加油 / 放量 / 资金 (未验证, P5 §3.2 强制不进 Strong)
    let reference_keywords = ["breakout", "空中加油", "放量", "资金净流入", "MACD金叉", "RSI"];
    if reference_keywords.iter().any(|kw| combined.contains(kw)) {
        return EvidenceTier::Reference;
    }
    // Theme: 仅产业链 / 板块热度 (没技术/资金信号)
    EvidenceTier::Theme
}

/// 硬门槛过滤 (P5 §3.3 — 用规则过滤, 用证据强度排序)
///
/// 剔除 (P5 §3.3):
/// 1. 已持仓 — 归 P0-4 管, 不进候选
/// 2. 停牌 — 用 v11 HALTED_CODES 缓存
/// 3. ST — 名字含 "*ST"/"ST"/"SST" 等
/// 4. 北交所 (8/4 开头) / 科创板 (688 开头) — 承接现有过滤
/// 5. 已涨停 (change_pct >= 9.9%) — 涨停次日接盘风险高
///
/// 输入 entries + 持仓 codes, 输出 过滤后的 entries.
pub fn filter_hard_gates(
    entries: Vec<CandidateEntry>,
    held_codes: &[String],
) -> Vec<CandidateEntry> {
    entries
        .into_iter()
        .filter(|e| {
            // 1. 剔除已持仓
            if held_codes.contains(&e.code) {
                return false;
            }
            // 2. 剔除停牌 (用 v11 HALTED_CODES 缓存)
            if is_halted(&e.code) {
                return false;
            }
            // 3. 剔除 ST (从 name 字段判断)
            if e.name.contains("ST") {
                return false;
            }
            // 4. 剔除北交所/科创板 (8/4/688 开头, 承接现有过滤)
            if e.code.starts_with('8')
                || e.code.starts_with('4')
                || e.code.starts_with("688")
            {
                return false;
            }
            // 5. 剔除已涨停 (change_pct >= 9.9%)
            if e.change_pct >= 9.9 {
                return false;
            }
            true
        })
        .collect()
}

/// 调 v11 HALTED_PERIODS 查 code 今天是否停牌 (用公开的 is_halted_period, 不依赖私有 is_halted)
fn is_halted(code: &str) -> bool {
    use chrono::Local;
    let today = Local::now().date_naive();
    crate::monitor::data_quality::is_halted_period(code, today)
}

#[cfg(test)]
mod tests_b {
    use super::*;

    /// 布林+MACD → Strong
    #[test]
    fn tier_boll_macd_is_strong() {
        let evidence = vec!["布林+MACD主升浪启动 (B方案, 已验证)".to_string()];
        assert_eq!(classify_tier(&evidence), EvidenceTier::Strong);
    }

    /// breakout 置信 75 → Reference (即使置信高, 未验证)
    #[test]
    fn tier_breakout_is_reference_not_strong() {
        let evidence = vec!["breakout 置信 78".to_string()];
        assert_eq!(
            classify_tier(&evidence),
            EvidenceTier::Reference,
            "breakout 未验证, P5 §3.2 红线: 绝不能进 Strong"
        );
    }

    /// 仅有产业链 / 板块热度 → Theme
    #[test]
    fn tier_industry_only_is_theme() {
        let evidence = vec!["机器人 (板块热度 88)".to_string()];
        assert_eq!(classify_tier(&evidence), EvidenceTier::Theme);
    }

    /// 硬门槛: 剔除已持仓
    #[test]
    fn hard_gate_exclude_held() {
        let entries = vec![
            CandidateEntry {
                code: "000001".to_string(),
                name: "A".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec!["布林+MACD".to_string()],
                current_price: 10.0,
                change_pct: 1.0,
            },
            CandidateEntry {
                code: "000002".to_string(),
                name: "B".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec!["布林+MACD".to_string()],
                current_price: 10.0,
                change_pct: 1.0,
            },
        ];
        let result = filter_hard_gates(entries, &["000001".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].code, "000002", "已持仓 (000001) 应被剔除");
    }

    /// 硬门槛: 剔除 ST/北交所/科创板/已涨停
    #[test]
    fn hard_gate_exclude_st_bse_star() {
        let entries = vec![
            // ST 票 — 剔除
            CandidateEntry {
                code: "000005".to_string(),
                name: "ST 测试".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec![],
                current_price: 10.0,
                change_pct: 1.0,
            },
            // 北交所 (8 开头) — 剔除
            CandidateEntry {
                code: "830799".to_string(),
                name: "北交所测试".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec![],
                current_price: 10.0,
                change_pct: 1.0,
            },
            // 科创板 (688 开头) — 剔除
            CandidateEntry {
                code: "688981".to_string(),
                name: "科创板测试".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec![],
                current_price: 10.0,
                change_pct: 1.0,
            },
            // 已涨停 (10%+) — 剔除
            CandidateEntry {
                code: "000999".to_string(),
                name: "涨停测试".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec![],
                current_price: 11.0,
                change_pct: 10.0,
            },
            // 正常票 — 保留
            CandidateEntry {
                code: "600000".to_string(),
                name: "正常".to_string(),
                sources: vec![CandidateSource::StockPick],
                tier: EvidenceTier::Strong,
                evidence: vec![],
                current_price: 10.0,
                change_pct: 1.0,
            },
        ];
        let result = filter_hard_gates(entries, &[]);
        assert_eq!(result.len(), 1, "只 1 只正常票应通过 5 个门槛");
        assert_eq!(result[0].code, "600000");
    }
}
