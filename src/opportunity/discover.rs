//! 机会发现 — 从产业链受益标的池中排除已持仓，按「逻辑硬度」排序输出 Top N。

use super::chain_mapper::{ChainHit, ChainSource};

/// 修复 F19 (2026-06-29 codex review): push_time 单调递增生成器.
///
/// 之前用 `chrono::Local::now().timestamp()` (秒级), 单次 `discover()` 调用所有
/// candidate 共享同一时间, BR-004 "同分按 push_time 升序" 次级排序是死代码 (production
/// 几乎不会触发).
///
/// 修法: 静态 AtomicI64 计数器, 每次 discover() 调用 +1, 给每个 candidate 分配单调递增
/// 时间戳 (单位 ms). 起始值用 `Local::now().timestamp_millis()` 让早期数字仍然有意义.
use std::sync::atomic::{AtomicI64, Ordering};
static PUSH_TIME_COUNTER: AtomicI64 = AtomicI64::new(0);

fn next_push_time() -> i64 {
    // 首次调用初始化为当前时间 (ms), 之后每次 +1 保证单调
    let prev = PUSH_TIME_COUNTER.load(Ordering::Relaxed);
    let next = if prev == 0 {
        chrono::Local::now().timestamp_millis()
    } else {
        prev + 1
    };
    PUSH_TIME_COUNTER.store(next, Ordering::Relaxed);
    next
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub code: String,
    pub name: String,
    pub chain: String,
    pub logic: String,
    pub score: f64,
    pub price_note: String, // "已启动+5.2% 追高风险" or ""
    pub reason_summary: String,
    /// 修复 F15 (2026-06-29 BR-004): 发布时间, 用于"同分按发布时间升序"次级排序.
    /// discover() 内填 Local::now().timestamp(). 老调用方 (e.g. 测试) 不填时为 0,
    /// 排序时 0 < 实际时间, 所以老测试 case 会排在新 case 前面 — 可观测的回归.
    pub push_time: i64,
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
    //     拆分后链路分上限 (source + keyword + fund + position):
    //       Rule max      = 10 + 9 + 10 + 5  = 34
    //       AI max        =  6 + 9 + 10 + 5  = 30
    //       AI 降级 max   =  0 + 9 + 10 + 5  = 24
    //     Rule vs AI 差 4 分 (逻辑硬度项), 配合资金/位置分才能拉开真实差距。
    // Refs: AGENTS §2.9 设计矛盾禁令 (边界证明)
    let source_score = match hit.source {
        ChainSource::Rule => 10.0,
        ChainSource::Ai => 6.0,
        ChainSource::AiDegraded => 0.0,
        // B-002 板块联动: 基于实时东财板块数据, 置信度接近 Rule 但略低 (无 100+ 测试回灌)
        ChainSource::Board => 8.0,
    };

    // ② 产业链位置/匹配强度：命中关键词越多越硬
    let keyword_score = (hit.keywords.len() as f64).min(3.0) * 3.0;

    // ③ 资金验证：板块主力净占比（缺数据记 0，不臆测）
    let fund_score = hit
        .fund_flow_pct
        .map(|f| f.clamp(-10.0, 10.0))
        .unwrap_or(0.0);

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
        ChainSource::Board => "板块联动",
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

/// 修复 v9.2 BR-001: 同一只票近 3 个日历日最多推送 1 次
/// (注: 实际按日历日计算, 非交易日 — 见 业务规则清单-registry.md BR-001 YAGNI 说明)
/// 修复 v9.2 M1 性能: 改成批量查询 (HashSet O(1)) 替代每次 sync DB round-trip.
/// 旧版本 `is_recently_pushed` 每个 stock 1 次 SQLite 查询, N×M 个 query 阻塞
/// async runtime (discover 被 run_opportunity_scan / run_post_close_candidates 调).
fn load_recently_pushed_codes(
    candidate_codes: &[String],
    days: i64,
) -> std::collections::HashSet<String> {
    let Some(db) = crate::database::DatabaseManager::try_get() else {
        return std::collections::HashSet::new(); // DB 未初始化 → 不阻断
    };
    match db.count_recent_pushes_batch(candidate_codes, days) {
        Ok(set) => set,
        Err(e) => {
            log::warn!(
                "[Discover] count_recent_pushes_batch 失败: {}, BR-001 放行",
                e
            );
            std::collections::HashSet::new()
        }
    }
}

/// 从产业链命中结果中发现新标的，按逻辑硬度排序输出 Top N。
pub fn discover(hits: &[ChainHit], exclude_codes: &[String], top_n: usize) -> Vec<Candidate> {
    let exclude: std::collections::HashSet<&str> =
        exclude_codes.iter().map(|c| c.as_str()).collect();
    let mut scored: Vec<(f64, Candidate)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_codes: Vec<String> = Vec::new();

    // 第一遍: 收集所有 candidate codes (dedup) 给批量 BR-001 查询用
    for hit in hits {
        if hit.stocks.is_empty() {
            continue;
        }
        for s in &hit.stocks {
            if exclude.contains(s.code.as_str()) {
                continue;
            }
            let market_code = market_rule_code(&s.code);
            if market_code.starts_with('8')
                || market_code.starts_with('4')
                || market_code.starts_with("688")
            {
                continue;
            }
            if !seen.insert(s.code.clone()) {
                continue;
            } // 去重
            all_codes.push(s.code.clone());
        }
    }

    // 一次批量查 BR-001 (近 3 日历日已推), 替代 N 次 sync DB round-trip
    let recently_pushed = load_recently_pushed_codes(&all_codes, 3);

    // 第二遍: 真正的发现循环 (BR-001 用 HashSet O(1) 查, 不再 sync DB)
    seen.clear(); // 重置 for 第二遍
    for hit in hits {
        if hit.stocks.is_empty() {
            continue;
        }

        for s in &hit.stocks {
            if exclude.contains(s.code.as_str()) {
                continue;
            }
            let market_code = market_rule_code(&s.code);
            if market_code.starts_with('8')
                || market_code.starts_with('4')
                || market_code.starts_with("688")
            {
                continue;
            }
            if !seen.insert(s.code.clone()) {
                continue;
            } // 去重

            // BR-001: 同一只票近 3 日历日已推 → 跳过 (HashSet O(1) 查, 0 次 DB)
            if recently_pushed.contains(&s.code) {
                log::debug!("[Discover] {} 近 3 日已推过, 跳过 (BR-001)", s.code);
                continue;
            }

            let score_breakdown = logic_hardness(hit, s);
            let score = score_breakdown.total();
            scored.push((
                score,
                Candidate {
                    code: s.code.clone(),
                    name: s.name.clone(),
                    chain: hit.chain.clone(),
                    logic: hit.logic.clone(),
                    score,
                    price_note: price_note(s),
                    reason_summary: reason_summary(hit, s, score_breakdown),
                    push_time: next_push_time(),
                },
            ));
        }
    }

    // 按逻辑硬度降序
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(top_n).map(|(_, c)| c).collect()
}

fn market_rule_code(code: &str) -> &str {
    #[cfg(test)]
    {
        code.strip_prefix("TEST_CODE_").unwrap_or(code)
    }
    #[cfg(not(test))]
    {
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn si(code: &str) -> crate::opportunity::chain_mapper::StockInfo {
        crate::opportunity::chain_mapper::StockInfo {
            code: code.into(),
            name: format!("股票{}", code),
            change_pct: 0.0,
            vol_ratio: 1.0,
        }
    }

    fn si_full(
        code: &str,
        change_pct: f64,
        vol_ratio: f64,
    ) -> crate::opportunity::chain_mapper::StockInfo {
        crate::opportunity::chain_mapper::StockInfo {
            code: code.into(),
            name: format!("股票{}", code),
            change_pct,
            vol_ratio,
        }
    }

    #[test]
    fn test_exclude_already_owned() {
        let hits = vec![ChainHit {
            chain: "AI硬件-PCB".into(),
            keywords: vec!["PCB".into()],
            logic: "PCB涨价".into(),
            stocks: vec![
                si("TEST_CODE_002579"),
                si("TEST_CODE_002938"),
                si("TEST_CODE_002916"),
            ],
            source: crate::opportunity::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: None,
            board_code: None,
            board_change_pct: None,
        }];
        let candidates = discover(&hits, &["TEST_CODE_002579".to_string()], 3);
        assert_eq!(candidates.len(), 2);
        assert!(!candidates.iter().any(|c| c.code == "TEST_CODE_002579"));
    }

    // 修复 F19 (2026-06-29 codex review): 验证 next_push_time 单调递增,
    // 让 BR-004 "同分按 push_time 升序" 次级排序在生产路径真生效.
    #[test]
    fn test_push_time_monotonic_incrementing() {
        let t1 = super::next_push_time();
        let t2 = super::next_push_time();
        let t3 = super::next_push_time();
        assert!(t2 > t1, "push_time 应单调递增: t1={t1} t2={t2}");
        assert!(t3 > t2, "push_time 应单调递增: t2={t2} t3={t3}");
        // 同一秒内调用也应 +1 (不会卡在同一时间戳)
        assert_eq!(t3 - t1, 2, "三次连续调用应差 2 ms (atomic counter 递增)");
    }

    // 修复 F19 (2026-06-29 codex review): discover() 多次调用 next_push_time() 严格递增.
    // 注: discover() 内部 sort_by 只按 score (不放 push_time), 实际 BR-004 同分排序
    // 在 run_post_close_candidates 的 PostCloseCandidate sort_by (tests/ranking.rs 已覆盖).
    // 这里只验证 next_push_time 单调 (counter atomic +1).
    #[test]
    fn test_discover_push_time_distinct_per_candidate() {
        let hits = vec![ChainHit {
            chain: "AI硬件-PCB".into(),
            keywords: vec!["PCB".into()],
            logic: "PCB涨价".into(),
            stocks: (0..10).map(|i| si(&format!("PCB{:06}", i))).collect(),
            source: crate::opportunity::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: None,
            board_code: None,
            board_change_pct: None,
        }];
        let candidates = discover(&hits, &[], 10);
        assert_eq!(candidates.len(), 10);
        // 用 HashSet 验证 push_time 都不同 (排除 stable sort 重排影响)
        let unique_count = candidates
            .iter()
            .map(|c| c.push_time)
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(
            unique_count, 10,
            "10 个 candidate push_time 应全部唯一 (atomic counter +1), 实际 {} 个",
            unique_count
        );
    }

    #[test]
    fn test_filter_st_stock() {
        let hits = vec![ChainHit {
            chain: "测试".into(),
            keywords: vec!["测试".into()],
            logic: "测试".into(),
            stocks: vec![
                si("TEST_CODE_002938"),
                si("TEST_CODE_400001"),
                si("TEST_CODE_800001"),
                si("TEST_CODE_688001"),
            ],
            source: crate::opportunity::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: None,
            board_code: None,
            board_change_pct: None,
        }];
        let candidates = discover(&hits, &[], 10);
        // 002938 中小板保留，其余北交所/科创板被过滤
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].code, "TEST_CODE_002938");
    }

    #[test]
    fn test_rank_by_fund_validation() {
        // 同等关键词，资金验证更强的产业链其标的排序靠前
        let weak = ChainHit {
            chain: "弱链".into(),
            keywords: vec!["A".into()],
            logic: "x".into(),
            stocks: vec![si("TEST_CODE_002001")],
            source: ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: Some(0.5),
            board_code: None,
            board_change_pct: None,
        };
        let strong = ChainHit {
            chain: "强链".into(),
            keywords: vec!["B".into()],
            logic: "y".into(),
            stocks: vec![si("TEST_CODE_002002")],
            source: ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: Some(8.0),
            board_code: None,
            board_change_pct: None,
        };
        let candidates = discover(&[weak, strong], &[], 2);
        assert_eq!(candidates[0].code, "TEST_CODE_002002"); // 强资金验证排第一
    }

    #[test]
    fn test_low_position_beats_chased() {
        // 低位放量卡位 优于 已启动追高
        let hit = ChainHit {
            chain: "链".into(),
            keywords: vec!["A".into()],
            logic: "x".into(),
            stocks: vec![
                si_full("TEST_CODE_002003", 9.0, 3.0),
                si_full("TEST_CODE_002004", 1.0, 1.5),
            ],
            source: ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: Some(3.0),
            board_code: None,
            board_change_pct: None,
        };
        let candidates = discover(&[hit], &[], 2);
        assert_eq!(candidates[0].code, "TEST_CODE_002004"); // 低位卡位排第一
        assert!(candidates[0].price_note.contains("卡位"));
        // 追高标的带风险提示
        let chased = candidates
            .iter()
            .find(|c| c.code == "TEST_CODE_002003")
            .unwrap();
        assert!(chased.price_note.contains("追高风险"));
    }
}
