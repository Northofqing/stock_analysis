//! 新闻 AI 分析器（Phase 1.6）。
//!
//! 路径A（机会发现）：entity_linker 候选池 → AI 打分筛选（做选择题，不做填空题）
//! 路径B（持仓深研）：消息 + BIAS/筹码/涨跌停 → AI 影响评估（不输出操作指令）
//!
//! 硬约束：
//! - 盘中 Quick 模式 <3s，盘后 Deep 模式 <15s，超时跳过
//! - 所有 AI 建议进 prediction_tracker（tag=AI_News_Quick）
//! - AtomicBool 单任务锁防异步雪崩
//! - 代码校验闸拦截 LLM 幻觉
//! - 告警带【AI舆情研判-仅供参考】前缀

use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel};
use crate::monitor::entity_linker::{EntityHit, EntityLinker};
use chrono::Local;
use log::warn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ── 异步雪崩防护 ──

static AI_TASK_RUNNING: AtomicBool = AtomicBool::new(false);

// ── 输出结构 ──

#[derive(Debug, Clone)]
pub struct Opportunity {
    pub code: String,
    pub name: String,
    pub reason: String,
    pub direction: String,  // "买入" / "回避"
    pub confidence: u8,     // 0-100
    pub horizon: String,    // "日内" / "1-3天" / "1周+"
}

#[derive(Debug, Clone)]
pub struct PositionAnalysis {
    pub code: String,
    pub name: String,
    pub impact: String,      // "重大利空" / "偏空" / "中性" / "偏多" / "重大利好"
    pub confidence: u8,
    pub uncertainty: String,
    pub core_logic: String,
}

// ── NewsAIAnalyzer ──

pub struct NewsAIAnalyzer {
    linker: EntityLinker,
    analyzer: GeminiAnalyzer,
    available: bool,
    /// 幻觉率追踪
    hallucination_count: u64,
    total_code_outputs: u64,
}

impl NewsAIAnalyzer {
    pub fn new() -> Self {
        let analyzer = GeminiAnalyzer::from_env();
        let available = analyzer.is_available();
        let mut linker = EntityLinker::new();
        // 加载持仓
        if let Ok(db) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            if let Ok(positions) = db.get_all_open_positions() {
                for p in &positions {
                    linker.register_position(&p.code, &p.name);
                }
            }
        }
        Self { linker, analyzer, available, hallucination_count: 0, total_code_outputs: 0 }
    }

    fn available(&self) -> bool { self.available }

    // ═══════════════════════════════════════════════════════════
    // 路径A：机会发现（entity_linker 候选池 → AI 打分）
    // ═══════════════════════════════════════════════════════════

    pub async fn discover_opportunities(&mut self, flash_titles: &[String]) -> Vec<AlertEvent> {
        if flash_titles.is_empty() || !self.available() { return vec![]; }
        if AI_TASK_RUNNING.swap(true, Ordering::SeqCst) {
            warn!("[NewsAI] 上次扫描未完成，跳过本次");
            return vec![];
        }

        let result = tokio::time::timeout(Duration::from_secs(3), async {
            // Step 1: entity_linker 从快讯中初筛候选
            let full_text = flash_titles.join("\n");
            let raw_hits = self.linker.link(&full_text);
            // 去重+限10只
            let mut seen = std::collections::HashSet::new();
            let mut candidates: Vec<&EntityHit> = Vec::new();
            for h in &raw_hits {
                if seen.insert(&h.code) { candidates.push(h); }
                if candidates.len() >= 10 { break; }
            }

            if candidates.is_empty() {
                return vec![];
            }

            // Step 2: 构建候选池文本
            let mut cand_text = String::new();
            for h in &candidates {
                cand_text.push_str(&format!("{}|{}|匹配原因:{}|置信度:{:.0}%\n",
                    h.code, h.name, h.reason, h.confidence * 100.0));
            }

            let prompt = format!(
                "你是A股短线策略师。根据最新快讯评估候选标的。\n\n<快讯>\n{}\n</快讯>\n\n<候选标的>\n{}\n</候选标的>\n\n要求：1.从候选池选0-3个有催化的 2.格式：代码|名称|逻辑|买入/回避|置信度|持续期 3.已涨停不推 4.没好机会输出\"无\"",
                flash_titles.join("\n"), cand_text
            );

            match self.analyzer.call_api_mode(&prompt, "你是A股策略师,只输出格式化的选股结果", AgentMode::Quick).await {
                Ok(text) => self.parse_opportunities(&text, &candidates),
                Err(e) => { warn!("[NewsAI] 机会发现失败: {}", e); vec![] }
            }
        }).await;

        AI_TASK_RUNNING.store(false, Ordering::SeqCst);
        result.unwrap_or_else(|_| {
            AI_TASK_RUNNING.store(false, Ordering::SeqCst);
            warn!("[NewsAI] 机会发现超时");
            vec![]
        })
    }

    fn parse_opportunities(&mut self, text: &str, candidates: &[&EntityHit]) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line == "无" || line.starts_with('#') { continue; }
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() < 5 { continue; }

            let code = parts[0];
            let name = parts.get(1).unwrap_or(&"");
            self.total_code_outputs += 1;

            // 代码校验闸
            let validated = validate_llm_code(code, name);
            if validated.is_none() {
                self.hallucination_count += 1;
                warn!("[NewsAI] 校验闸拦截: {}|{}", code, name);
                continue;
            }

            // 确认在候选池内
            if !candidates.iter().any(|c| c.code == code) {
                warn!("[NewsAI] 代码{}不在候选池,跳过", code);
                continue;
            }

            let reason = parts.get(2).unwrap_or(&"").to_string();
            let direction = parts.get(3).unwrap_or(&"").to_string();
            let confidence = parts.get(4).unwrap_or(&"50").parse::<u8>().unwrap_or(50);
            let horizon = parts.get(5).unwrap_or(&"1-3天").to_string();

            // 写入 prediction_tracker（Shadow mode）
            let _ = crate::monitor::prediction::save_prediction(
                None, Some(code), if direction.contains("买入") { "看多" } else { "看空" },
                confidence as f64, Some(&reason),
            );

            events.push(AlertEvent {
                level: AlertLevel::Info,
                category: AlertCategory::FlashNews,
                code: code.to_string(),
                name: name.to_string(),
                message: format!("【AI舆情研判-仅供参考】{} | {} | 置信度{}% | {}",
                    direction, reason, confidence, horizon),
                detail: AlertDetail {
                    price: None, change_pct: None, volume_ratio: None,
                    main_flow_yi: None, threshold: None,
                    news_title: Some(reason.clone()),
                    news_summary: None, ai_decision: None,
                    t1_locked: false,
                    extra: Some(format!("AI推荐,置信度{}%,持续期{}", confidence, horizon)),
                },
                triggered_at: Local::now(),
            });
        }
        events
    }

    // ═══════════════════════════════════════════════════════════
    // 路径B：持仓深研（消息 + 技术面 → 影响评估）
    // ═══════════════════════════════════════════════════════════

    pub async fn analyze_position_news(
        &self, code: &str, name: &str, news_text: &str,
        price: f64, pe: f64, roe: f64, chg_5d: f64,
        alignment: &str, bias_5: f64, limit_status: &str,
        chip_position: &str, hard_stop: f64,
    ) -> Option<AlertEvent> {
        if !self.available() { return None; }

        let prompt = format!(
            "你是A股风控分析师。分析消息对持仓股的影响。\n\n股票：{}({})\n最新价：{:.2} | PE：{:.1} | ROE：{:.1}%\n近5日：{:+.1}% | 均线：{}\n5日乖离(BIAS_5)：{:+.1}% | 状态：{}\n筹码位置：{}\n硬止损价：{:.2}\n\n消息：\n{}\n\n输出（严格格式）：\n影响判断：重大利空/偏空/中性/偏多/重大利好\n置信度：0-100\n不确定点：（一句话）\n核心逻辑：（一句话）\n\n限制：已涨停禁建议加仓，已跌停禁建议减仓，高开>5%建议等回落。禁输出操作指令。",
            name, code, price, pe, roe, chg_5d, alignment, bias_5, limit_status, chip_position, hard_stop, news_text
        );

        let result = tokio::time::timeout(Duration::from_secs(3), async {
            self.analyzer.call_api_mode(&prompt, "你是A股风控分析师,只输出结构化的影响评估", AgentMode::Quick).await
        }).await;

        match result {
            Ok(Ok(text)) => self.parse_position_analysis(code, name, &text, news_text),
            Ok(Err(e)) => { warn!("[NewsAI] 持仓分析失败: {}", e); None },
            Err(_) => { warn!("[NewsAI] 持仓分析超时"); None },
        }
    }

    fn parse_position_analysis(&self, code: &str, name: &str, text: &str, news_text: &str) -> Option<AlertEvent> {
        let mut impact = "中性";
        let mut confidence = 50u8;
        let mut uncertainty = "";
        let mut core_logic = "";

        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("影响判断：") || line.starts_with("影响判断:") {
                impact = line.split(['：', ':']).last().unwrap_or("中性").trim();
            }
            if line.starts_with("置信度：") || line.starts_with("置信度:") {
                confidence = line.split(['：', ':']).last().unwrap_or("50").trim().parse().unwrap_or(50);
            }
            if line.starts_with("不确定点：") || line.starts_with("不确定点:") {
                uncertainty = line.split(['：', ':']).last().unwrap_or("").trim();
            }
            if line.starts_with("核心逻辑：") || line.starts_with("核心逻辑:") {
                core_logic = line.split(['：', ':']).last().unwrap_or("").trim();
            }
        }

        // 消息×价格交叉确认
        let level = match impact {
            "重大利空" => AlertLevel::Important, // 不直接紧急，需价格二次确认
            "偏空" => AlertLevel::Info,
            "重大利好" => AlertLevel::Important,
            _ => return None, // 中性/偏多不发告警
        };

        // 写入 prediction_tracker
        let _ = crate::monitor::prediction::save_prediction(
            None, Some(code), if impact.contains("利好") { "看多" } else { "看空" },
            confidence as f64, Some(core_logic),
        );

        let msg = format!(
            "【AI舆情研判-仅供参考】{}({}) | 影响:{} | 置信度:{}% | {}\n不确定点:{}",
            name, code, impact, confidence, core_logic, uncertainty
        );

        Some(AlertEvent {
            level,
            category: AlertCategory::FlashNews,
            code: code.to_string(), name: name.to_string(),
            message: msg,
            detail: AlertDetail {
                price: None, change_pct: None, volume_ratio: None,
                main_flow_yi: None, threshold: None,
                news_title: Some(news_text.chars().take(100).collect()),
                news_summary: None, ai_decision: None,
                t1_locked: false,
                extra: Some(format!("AI影响评估:{},置信度:{}%", impact, confidence)),
            },
            triggered_at: Local::now(),
        })
    }

    // ═══════════════════════════════════════════════════════════
    // 一句话交易决策（超快模式，<2s，失败静默降级）
    // ═══════════════════════════════════════════════════════════

    pub async fn quick_decision(&self, title: &str, code: &str, name: &str) -> Option<String> {
        // 先尝试 AI 调用
        if self.available() {
            let prompt = format!(
                "你是A股交易助手。根据快讯给出一句话交易建议（≤40字）。\n\
                 股票：{}({})\n快讯：{}\n\
                 要求：简洁可操作，用'关注/回避/持有/观察'等中性词，不输出买卖指令。直接输出一句话。",
                name, code, title
            );

            let result = tokio::time::timeout(Duration::from_secs(6), async {
                self.analyzer.call_api_mode(
                    &prompt, "你是A股交易助手,只输出一句话建议", AgentMode::Quick,
                ).await
            }).await;

            match result {
                Ok(Ok(text)) => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && trimmed.chars().count() <= 80 {
                        log::info!("[NewsAI] quick_decision(AI): {}", trimmed);
                        return Some(trimmed.to_string());
                    }
                    if trimmed.chars().count() > 80 {
                        log::warn!("[NewsAI] AI输出过长({}字), 降级关键词", trimmed.chars().count());
                    }
                }
                Ok(Err(e)) => log::warn!("[NewsAI] AI失败: {}, 降级关键词", e),
                Err(_) => log::warn!("[NewsAI] AI超时(>6s), 降级关键词"),
            }
        }

        // 关键词规则回退：AI 不可用/失败/超时 → 规则兜底
        keyword_decision(title)
    }

    // ═══════════════════════════════════════════════════════════
    // 自监控
    // ═══════════════════════════════════════════════════════════

    pub fn hallucination_rate(&self) -> f64 {
        if self.total_code_outputs == 0 { 0.0 }
        else { self.hallucination_count as f64 / self.total_code_outputs as f64 }
    }

    pub fn stats(&self) -> String {
        format!("AI分析 | 幻觉率:{:.0}%({}/{})",
            self.hallucination_rate() * 100.0, self.hallucination_count, self.total_code_outputs)
    }
}

// ═══════════════════════════════════════════════════════════
// 关键词规则回退（AI 不可用时的兜底决策）
// ═══════════════════════════════════════════════════════════

fn keyword_decision(title: &str) -> Option<String> {
    // 利空关键词 → 回避
    const BEARISH: &[&str] = &[
        "立案", "ST风险", "退市", "破产", "清算", "无法表示意见",
        "减持", "业绩预亏", "商誉减值", "重大诉讼", "强制退市",
        "暂停上市", "失联", "冻结", "监管函", "警示函", "责令改正",
        "问询函",
    ];
    // 利好关键词 → 关注
    const BULLISH: &[&str] = &[
        "中标", "回购", "增持", "业绩预增", "重组", "重大合同",
        "战略合作", "获得批文", "高送转", "解除质押",
    ];
    // 中性关键词 → 观察
    const NEUTRAL: &[&str] = &[
        "质押", "解禁", "股东大会", "董事会", "变更",
    ];

    for &kw in BEARISH {
        if title.contains(kw) {
            log::info!("[NewsAI] 关键词命中(利空): {}", kw);
            let advice = match kw {
                "立案" | "ST风险" | "退市" | "破产" => "重大利空，建议回避并关注后续公告",
                "减持" => "减持短期承压，关注减持比例和进度",
                "质押" if !title.contains("解除") => "质押需关注平仓风险，密切跟踪",
                "冻结" => "资产冻结属利空，回避等明朗",
                "监管函" | "警示函" | "问询函" => "监管关注属偏空，暂持观察",
                "业绩预亏" | "商誉减值" => "业绩利空，短期回避等企稳",
                _ => "偏空消息，建议回避，等待风险释放",
            };
            return Some(advice.to_string());
        }
    }
    for &kw in BULLISH {
        if title.contains(kw) {
            log::info!("[NewsAI] 关键词命中(利好): {}", kw);
            let advice = match kw {
                "中标" | "重大合同" => "利好催化，关注合同金额和业绩贡献",
                "回购" => "回购属利好信号，关注回购规模和价格",
                "增持" => "增持提振信心，关注增持比例",
                "业绩预增" => "业绩利好，关注增长幅度和持续性",
                "重组" | "战略合作" => "重组/合作催化，关注方案落地进度",
                _ => "利好消息，可关注后续催化",
            };
            return Some(advice.to_string());
        }
    }
    for &kw in NEUTRAL {
        if title.contains(kw) {
            log::info!("[NewsAI] 关键词命中(中性): {}", kw);
            return Some("中性消息，持续观察，暂不操作".to_string());
        }
    }
    // 无关键词命中
    Some("消息需进一步分析，暂持观望".to_string())
}

// ═══════════════════════════════════════════════════════════
// 代码校验闸
// ═══════════════════════════════════════════════════════════

fn validate_llm_code(code: &str, name: &str) -> Option<String> {
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) { return None; }
    if code.starts_with('8') || code.starts_with('4') || code.starts_with('9') { return None; }
    if name.contains("ST") || name.contains("退") { return None; }
    Some(code.to_string())
}

impl Default for NewsAIAnalyzer {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_code_valid() {
        assert!(validate_llm_code("000547", "航天发展").is_some());
        assert!(validate_llm_code("600519", "贵州茅台").is_some());
    }

    #[test]
    fn test_validate_code_invalid() {
        assert!(validate_llm_code("00054", "测试").is_none());   // 5位
        assert!(validate_llm_code("800547", "北交所").is_none()); // 北交所
        assert!(validate_llm_code("000001", "ST测试").is_none()); // ST
    }

    #[test]
    fn test_parse_opportunities_empty() {
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test_ai.db")));
        let mut ai = NewsAIAnalyzer::new();
        let events = ai.parse_opportunities("无", &[]);
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_position_basic() {
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test_ai.db")));
        let ai = NewsAIAnalyzer::new();
        let text = "影响判断：偏空\n置信度：70\n不确定点：减持比例未明确\n核心逻辑：大股东减持通常短期承压";
        let event = ai.parse_position_analysis("000001", "测试", text, "减持公告");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.level, AlertLevel::Info);
        assert!(e.message.contains("AI舆情研判"));
        assert!(e.message.contains("偏空"));
    }

    #[test]
    fn test_hallucination_rate() {
        let mut ai = NewsAIAnalyzer::new();
        ai.total_code_outputs = 10;
        ai.hallucination_count = 3;
        assert!((ai.hallucination_rate() - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_keyword_decision_bearish() {
        let d = keyword_decision("关于收到立案调查通知书的公告").unwrap();
        assert!(d.contains("回避"));
        let d = keyword_decision("关于持股5%以上股东减持股份的公告").unwrap();
        assert!(d.contains("减持"));
    }

    #[test]
    fn test_keyword_decision_bullish() {
        let d = keyword_decision("关于收到中标通知书的公告").unwrap();
        assert!(d.contains("利好"));
        let d = keyword_decision("关于回购公司股份方案的公告").unwrap();
        assert!(d.contains("回购"));
    }

    #[test]
    fn test_keyword_decision_neutral() {
        let d = keyword_decision("关于召开2025年股东大会的通知").unwrap();
        assert!(d.contains("中性"));
    }

    #[test]
    fn test_keyword_decision_unknown() {
        let d = keyword_decision("关于公司日常经营情况的说明").unwrap();
        assert!(d.contains("观望"));
    }

    #[test]
    fn test_validate_code_llm_output_ok() {
        // 初始化DB（避免NewsAIAnalyzer::new中panic）
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test_ai.db")));
        let mut ai = NewsAIAnalyzer::new();
        // 构造候选池（含目标代码以通过"在候选池内"检查）
        let hit = EntityHit { code: "000547".into(), name: "航天发展".into(), confidence: 0.9, reason: "匹配".into() };
        let candidates = vec![&hit];
        let text = "000547|航天发展|低空经济催化|买入|75|1-3天";
        let events = ai.parse_opportunities(text, &candidates);
        assert_eq!(ai.total_code_outputs, 1);
        assert_eq!(ai.hallucination_count, 0);
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("AI舆情研判"));
    }
}
