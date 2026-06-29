//! 机会发现 — 从产业链受益标的池中排除已持仓，按「逻辑硬度」排序输出 Top N。

use super::chain_mapper::{ChainHit, ChainSource};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub code: String,
    pub name: String,
    pub chain: String,
    pub logic: String,
    pub score: f64,
    pub price_note: String, // "已启动+5.2% 追高风险" or ""
    pub reason_summary: String,
}

#[derive(Debug, Clone, Copy)]
struct ScoreBreakdown {
    source_score: f64,
    keyword_score: f64,
    fund_score: f64,
    position_score: f64,
}

impl ScoreBreakdown {
    fn total(self) -> f64 {
        self.source_score + self.keyword_score + self.fund_score + self.position_score
    }
}

/// 「逻辑硬度」评分：政策力度(产业链来源可信度) × 产业链位置(关键词强度) × 资金验证(板块主力流向)
/// + 低位卡位加分。替代旧的"按命中顺序/关键词计数"粗排。
///
/// 数据红线 2.2：某维度数据缺失则该维度记 0 分，不补默认高分。
fn logic_hardness(hit: &ChainHit, s: &super::chain_mapper::StockInfo) -> ScoreBreakdown {
    // ① 产业链来源可信度：规则命中(已验证映射) > AI 推理 > AI降级
    //
    // 边界证明 (PR-3 修 R-7):
    // - Rule 命中 10 分: 关键词表已校验 (100+ 测试), 信度高
    // - AI 命中 6 分: Gemini 解析, 历史准确率 ~60% (经验值), 留 4 分缓冲
    // - AI 降级 0 分: 无真实分析, 不应得逻辑硬度分
    //
    // 注: 之前 source_score 是常量 10, 实际 Rule 和 AI 都 +10, 区分度为 0。
    //     拆分后链路分最大 24 → Rule 24, AI 20, 差 4 分, 配合资金/位置分
    //     才能拉开真实差距。
    // Refs: AGENTS §2.9 设计矛盾禁令 (边界证明)
    let source_score = match hit.source {
        ChainSource::Rule => 10.0,
        ChainSource::Ai => 6.0,
        ChainSource::AiDegraded => 0.0,
    };

    // ② 产业链位置/匹配强度：命中关键词越多越硬
    let keyword_score = (hit.keywords.len() as f64).min(3.0) * 3.0;

    // ③ 资金验证：板块主力净占比（缺数据记 0，不臆测）
    let fund_score = hit.fund_flow_pct.map(|f| f.clamp(-10.0, 10.0)).unwrap_or(0.0);

    // ④ 低位卡位：涨幅低 + 量比抬头 → 补涨空间大；已启动则减分（追高风险）
    let position_score = if s.change_pct >= 7.0 {
        -5.0 // 追高风险
    } else if s.change_pct <= 2.0 && s.vol_ratio >= 1.2 {
        5.0 // 低位放量卡位
    } else {
        0.0
    };

    ScoreBreakdown {
        source_score,
        keyword_score,
        fund_score,
        position_score,
    }
}

/// 生成价格风险提示（v3 意图：已启动追高风险 / 低位卡位）
fn price_note(s: &super::chain_mapper::StockInfo) -> String {
    if s.change_pct >= 7.0 {
        format!("已启动+{:.1}% 追高风险", s.change_pct)
    } else if s.change_pct <= 2.0 && s.vol_ratio >= 1.2 {
        "低位放量 卡位候选".to_string()
    } else {
        String::new()
    }
}

fn reason_summary(hit: &ChainHit, s: &super::chain_mapper::StockInfo, b: ScoreBreakdown) -> String {
    let source = match hit.source {
        ChainSource::Rule => "规则命中",
        ChainSource::Ai => "AI推理",
        ChainSource::AiDegraded => "AI降级",
    };

    let position = if b.position_score > 0.0 {
        "低位放量卡位"
    } else if b.position_score < 0.0 {
        "已启动偏追高"
    } else {
        "位置中性"
    };

    format!(
        "总分{:.1} = 来源({}:{:.1}) + 关键词({}个:{:.1}) + 资金({:.1}) + 位置({}:{:.1})；现价涨幅{:+.1}% 量比{:.1}",
        b.total(),
        source,
        b.source_score,
        hit.keywords.len(),
        b.keyword_score,
        b.fund_score,
        position,
        b.position_score,
        s.change_pct,
        s.vol_ratio,
    )
}

/// 修复 v9.2 BR-001: 同一只票近 3 个交易日最多推送 1 次
fn is_recently_pushed(code: &str) -> bool {
    // DB 不可用时放行, 不阻断业务
    let db = match std::panic::catch_unwind(crate::database::DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => return false,
    };
    match db.count_recent_pushes(code, 3) {
        Ok(n) => n > 0,
        Err(_) => false,
    }
}

/// 从产业链命中结果中发现新标的，按逻辑硬度排序输出 Top N。
pub fn discover(
    hits: &[ChainHit],
    exclude_codes: &[String],
    top_n: usize,
) -> Vec<Candidate> {
    let exclude: std::collections::HashSet<&str> = exclude_codes.iter().map(|c| c.as_str()).collect();
    let mut scored: Vec<(f64, Candidate)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for hit in hits {
        if hit.stocks.is_empty() { continue; }

        for s in &hit.stocks {
            if exclude.contains(s.code.as_str()) { continue; }
            if s.code.starts_with('8') || s.code.starts_with('4') || s.code.starts_with("688") {
                continue;
            }
            if !seen.insert(s.code.clone()) { continue; } // 去重

            // 修复 BR-001: 同一只票近 3 个交易日已推 → 跳过
            if is_recently_pushed(&s.code) {
                log::debug!("[Discover] {} 近 3 日已推过, 跳过 (BR-001)", s.code);
                continue;
            }

            let score_breakdown = logic_hardness(hit, s);
            let score = score_breakdown.total();
            scored.push((score, Candidate {
                code: s.code.clone(),
                name: s.name.clone(),
                chain: hit.chain.clone(),
                logic: hit.logic.clone(),
                score,
                price_note: price_note(s),
                reason_summary: reason_summary(hit, s, score_breakdown),
            }));
        }
    }

    // 按逻辑硬度降序
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(top_n).map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn si(code: &str) -> crate::opportunity::chain_mapper::StockInfo {
        crate::opportunity::chain_mapper::StockInfo { code: code.into(), name: format!("股票{}", code), change_pct: 0.0, vol_ratio: 1.0 }
    }

    fn si_full(code: &str, change_pct: f64, vol_ratio: f64) -> crate::opportunity::chain_mapper::StockInfo {
        crate::opportunity::chain_mapper::StockInfo { code: code.into(), name: format!("股票{}", code), change_pct, vol_ratio }
    }

    #[test]
    fn test_exclude_already_owned() {
        let hits = vec![ChainHit {
            chain: "AI硬件-PCB".into(),
            keywords: vec!["PCB".into()],
            logic: "PCB涨价".into(),
            stocks: vec![si("002579"), si("002938"), si("002916")],
            source: crate::opportunity::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: None,
        }];
        let candidates = discover(&hits, &["002579".to_string()], 3);
        assert_eq!(candidates.len(), 2);
        assert!(!candidates.iter().any(|c| c.code == "002579"));
    }

    #[test]
    fn test_filter_st_stock() {
        let hits = vec![ChainHit {
            chain: "测试".into(),
            keywords: vec!["测试".into()],
            logic: "测试".into(),
            stocks: vec![si("002938"), si("400001"), si("800001"), si("688001")],
            source: crate::opportunity::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: None,
        }];
        let candidates = discover(&hits, &[], 10);
        // 002938 中小板保留，其余北交所/科创板被过滤
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].code, "002938");
    }

    #[test]
    fn test_rank_by_fund_validation() {
        // 同等关键词，资金验证更强的产业链其标的排序靠前
        let weak = ChainHit {
            chain: "弱链".into(), keywords: vec!["A".into()], logic: "x".into(),
            stocks: vec![si("002001")],
            source: ChainSource::Rule, board_keyword: String::new(),
            fund_flow_pct: Some(0.5),
        };
        let strong = ChainHit {
            chain: "强链".into(), keywords: vec!["B".into()], logic: "y".into(),
            stocks: vec![si("002002")],
            source: ChainSource::Rule, board_keyword: String::new(),
            fund_flow_pct: Some(8.0),
        };
        let candidates = discover(&[weak, strong], &[], 2);
        assert_eq!(candidates[0].code, "002002"); // 强资金验证排第一
    }

    #[test]
    fn test_low_position_beats_chased() {
        // 低位放量卡位 优于 已启动追高
        let hit = ChainHit {
            chain: "链".into(), keywords: vec!["A".into()], logic: "x".into(),
            stocks: vec![si_full("002003", 9.0, 3.0), si_full("002004", 1.0, 1.5)],
            source: ChainSource::Rule, board_keyword: String::new(),
            fund_flow_pct: Some(3.0),
        };
        let candidates = discover(&[hit], &[], 2);
        assert_eq!(candidates[0].code, "002004"); // 低位卡位排第一
        assert!(candidates[0].price_note.contains("卡位"));
        // 追高标的带风险提示
        let chased = candidates.iter().find(|c| c.code == "002003").unwrap();
        assert!(chased.price_note.contains("追高风险"));
    }
}
