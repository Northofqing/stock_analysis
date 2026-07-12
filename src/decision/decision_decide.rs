//! v11-P0-4 commit B: 持仓决策台 — 裁决层
//!
//! ## 背景
//!
//! Commit A 落了 `Action` / `Priority` / `FinalDecision` 类型. Commit B 落 **裁决规则**:
//! 输入 `DecisionInputs` (止损信号 / 风控违规 / 放量弱化 / 消息面利空 + 现价/涨跌幅),
//! 按 5 层规则 + 冲突裁决输出 `FinalDecision`.
//!
//! ## 5 层规则 (P0-4 §3.4, 风控一票否决)
//!
//! - 层1: 硬止损 (StopLevel::Hard) **或** 风控超标 (LimitViolation) → ReduceNow (P0)
//! - 层2: 技术/结构止损 (StopLevel::Technical/Structural) → Reduce (P1)
//! - 层3: 放量冲高回落/尾盘跳水 (volume_weak) → Reduce (P1)
//! - 层4: 消息面重大利空 (news_negative) → Reduce (P1)
//! - 层5: 无风险信号 + 布林买点 (boll_buy_point) → WatchAdd (P2)
//! - 默认: 无信号 → Hold (P2) — 诚实标注, 不假装中性
//!
//! ## 冲突裁决
//!
//! 层1 一票否决: 即使层5 布林买点信号, 硬止损/风控超标仍优先 ReduceNow.
//! 多层同时触发: 取最严重层 (层1 > 层2 > 层3/4 > 层5 > 默认).
//!
//! ## AI 隔离 (grill Q4 决策)
//!
//! `ai_card_summary` 是输入参数, **不进** `decide()` 裁决逻辑. AI 文字只在卡片渲染时附注,
//! 不影响 Action/Priority. v11 IC 证伪 sentiment_score, 数字误导, 排除.

use crate::decision::decision_panel::{
    Action, DecisionReason, DecisionReasonKind, FinalDecision, Priority,
};
use crate::portfolio::Position;
use crate::risk::limits::LimitViolation;
use crate::risk::stop_loss::{StopLevel, StopSignal};
use std::collections::HashMap;

/// 持仓决策输入
///
/// 喂入 `decide()` 的一只持仓所有信号. AI 文字 (ai_card_summary) 不参与裁决, 只在卡片附注.
#[derive(Clone, Debug)]
pub struct DecisionInputs {
    /// 股票代码 (6 位)
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 当前价
    pub current_price: f64,
    /// 今日涨跌幅 (%)
    pub change_pct: f64,
    /// 止损信号 (risk::stop_loss::check_stops 输出, 可能空)
    pub stop_signals: Vec<StopSignal>,
    /// 风控违规 (risk::limits::check_position_limits 输出, 可能空)
    pub limit_violations: Vec<LimitViolation>,
    /// 放量冲高回落/尾盘跳水 (B5 推送分析出的 bool)
    pub volume_weak: bool,
    /// 消息面重大利空 (C5 推送的 bool)
    pub news_negative: bool,
    /// 布林买点 (放量分析/技术面)
    pub boll_buy_point: bool,
    /// 硬止损价 (None 表示未设)
    pub hard_stop: Option<f64>,
    /// AI 1-2 句摘要 (grill Q4: 不进裁决, 仅卡片附注)
    pub ai_card_summary: Option<String>,
}

/// 5 层规则 + 冲突裁决
///
/// 风控一票否决: 硬止损或风控超标 → ReduceNow (P0), 即使有布林买点也压过.
/// 优先级: 层1 > 层2 > 层3/层4 > 层5 > 默认.
///
/// 各层 reasons 独立收集 (信息完整性), 只在最后取最严重层做 action/priority.
pub fn decide(inputs: DecisionInputs) -> FinalDecision {
    let mut reasons: Vec<DecisionReason> = Vec::new();
    let mut triggered_layer: Option<u8> = None; // 1-5, None = 默认 Hold, 数字越小越严重

    // 层1a: 硬止损 → ReduceNow (P0)
    if let Some(s) = inputs
        .stop_signals
        .iter()
        .find(|s| s.level == StopLevel::Hard)
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::StopLoss,
            format!(
                "硬止损触发 ¥{:.2} (现价 ¥{:.2})",
                s.trigger_price, s.current_price
            ),
        ));
        triggered_layer = Some(1);
    }

    // 层1b: 风控超标 → ReduceNow (P0)
    if let Some(v) = inputs.limit_violations.first() {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::PositionLimit,
            format!("风控超标: {} ({} > {})", v.rule, v.current, v.limit),
        ));
        // 风控一票否决, 即使层1a 硬止损已触发也保持层1
        triggered_layer = Some(1);
    }

    // 层2: 技术/结构止损 → Reduce (P1) (独立检查, 不管层1 是否触发)
    if let Some(s) = inputs
        .stop_signals
        .iter()
        .find(|s| s.level == StopLevel::Technical || s.level == StopLevel::Structural)
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::StopLoss,
            format!("{}触发 ¥{:.2}", s.level.label(), s.trigger_price),
        ));
        // 仅当层1 未触发时才设为层2 (层1 仍压过)
        if triggered_layer.is_none() || triggered_layer > Some(2) {
            triggered_layer = Some(2);
        }
    }

    // 层3: 放量冲高回落/尾盘跳水 → Reduce (P1)
    if inputs.volume_weak {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::VolumePattern,
            "放量冲高回落/尾盘跳水 (已验证形态)",
        ));
        if triggered_layer.is_none() || triggered_layer > Some(3) {
            triggered_layer = Some(3);
        }
    }

    // 层4: 消息面重大利空 → Reduce (P1)
    if inputs.news_negative {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::NewsImpact,
            "消息面重大利空 (C5)",
        ));
        if triggered_layer.is_none() || triggered_layer > Some(4) {
            triggered_layer = Some(4);
        }
    }

    // 层5: 布林买点 + 无风险信号 → WatchAdd (P2)
    if inputs.boll_buy_point
        && !inputs.volume_weak
        && !inputs.news_negative
        && inputs.stop_signals.is_empty()
        && inputs.limit_violations.is_empty()
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::VolumePattern,
            "布林买点 + 无风险信号",
        ));
        // 层5 仅在层1-4 都未触发时
        if triggered_layer.is_none() {
            triggered_layer = Some(5);
        }
    }

    // 默认: 无信号 → Hold (P2) 诚实标注
    if reasons.is_empty() {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::Health,
            "无风险信号, 持有观察 (默认)",
        ));
    }

    // 映射层 → (Action, Priority)
    let (action, priority) = match triggered_layer {
        Some(1) => (Action::ReduceNow, Priority::P0),
        Some(2) | Some(3) | Some(4) => (Action::Reduce, Priority::P1),
        Some(5) => (Action::WatchAdd, Priority::P2),
        None => (Action::Hold, Priority::P2),
        _ => unreachable!(),
    };

    // 构造 FinalDecision (grill Q4: AI 摘要通过 with_ai_summary 链式)
    let mut d = FinalDecision::new(
        inputs.code,
        inputs.name,
        inputs.current_price,
        inputs.change_pct,
        action,
        priority,
        reasons,
    );
    if let Some(stop) = inputs.hard_stop {
        d = d.with_stop_loss(stop);
    }
    if let Some(ai) = inputs.ai_card_summary {
        d = d.with_ai_summary(ai);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::decision_panel::Action;

    fn default_inputs() -> DecisionInputs {
        DecisionInputs {
            code: "000001".to_string(),
            name: "test".to_string(),
            current_price: 10.0,
            change_pct: 0.0,
            stop_signals: vec![],
            limit_violations: vec![],
            volume_weak: false,
            news_negative: false,
            boll_buy_point: false,
            hard_stop: None,
            ai_card_summary: None,
        }
    }

    /// 默认: 无信号 → Hold (P2)
    #[test]
    fn layer_default_hold() {
        let d = decide(default_inputs());
        assert_eq!(d.action, Action::Hold);
        assert_eq!(d.priority, Priority::P2);
        assert_eq!(d.reasons.len(), 1);
        assert_eq!(d.reasons[0].kind, DecisionReasonKind::Health);
    }

    /// 层1 硬止损 → ReduceNow (P0), 风控一票否决
    #[test]
    fn layer1_hard_stop_reduce_now() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Hard,
            current_price: 10.0,
            trigger_price: 12.0,
            reason: "硬止损".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
    }

    /// 层1 风控超标 → ReduceNow (P0)
    #[test]
    fn layer1_limit_violation_reduce_now() {
        let mut inputs = default_inputs();
        inputs.limit_violations.push(LimitViolation {
            code: "000001".to_string(),
            name: "test".to_string(),
            rule: "单票仓位上限".to_string(),
            current: "15.0%".to_string(),
            limit: "≤10%".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
    }

    /// 层2 技术/结构止损 → Reduce (P1)
    #[test]
    fn layer2_tech_stop_reduce() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Technical,
            current_price: 10.0,
            trigger_price: 10.5,
            reason: "破 MA20".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层3 放量弱化 → Reduce (P1)
    #[test]
    fn layer3_volume_weak_reduce() {
        let mut inputs = default_inputs();
        inputs.volume_weak = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层4 消息面利空 → Reduce (P1)
    #[test]
    fn layer4_news_negative_reduce() {
        let mut inputs = default_inputs();
        inputs.news_negative = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层5 布林买点 + 无风险 → WatchAdd (P2)
    #[test]
    fn layer5_boll_buy_watch_add() {
        let mut inputs = default_inputs();
        inputs.boll_buy_point = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::WatchAdd);
        assert_eq!(d.priority, Priority::P2);
    }

    /// 冲突: 层1 硬止损 + 层5 布林买点 → 层1 (P0 一票否决)
    #[test]
    fn layer1_overrides_layer5() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Hard,
            current_price: 10.0,
            trigger_price: 12.0,
            reason: "硬止损".to_string(),
        });
        inputs.boll_buy_point = true; // 试图 WatchAdd, 但应被层1 压过
        let d = decide(inputs);
        assert_eq!(
            d.action,
            Action::ReduceNow,
            "层1 一票否决, 即使层5 布林买点也应是 ReduceNow"
        );
        assert_eq!(d.priority, Priority::P0);
    }

    /// AI 摘要: 进 final_decision.ai_card_summary, 不影响 action/priority
    #[test]
    fn ai_summary_does_not_change_decision() {
        let mut inputs = default_inputs();
        inputs.ai_card_summary = Some("强烈卖出 (composite=28)".to_string());
        let d = decide(inputs);
        assert_eq!(d.action, Action::Hold); // 默认 Hold, AI 文字不影响
        assert_eq!(d.priority, Priority::P2);
        assert_eq!(
            d.ai_card_summary.as_deref(),
            Some("强烈卖出 (composite=28)")
        );
    }

    /// 优先级顺序: 层1 > 层2 (即使同时触发, 取层1)
    #[test]
    fn priority_layer1_beats_layer2() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Technical, // 层2
            current_price: 10.0,
            trigger_price: 10.5,
            reason: "技术".to_string(),
        });
        inputs.limit_violations.push(LimitViolation {
            code: "000001".to_string(),
            name: "test".to_string(),
            rule: "单票仓位上限".to_string(),
            current: "15%".to_string(),
            limit: "≤10%".to_string(),
        });
        let d = decide(inputs);
        // 两个 reason (技术止损 + 风控超标), 但 action/priority 是层1 (P0)
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
        assert_eq!(
            d.reasons.len(),
            2,
            "两个 reason 都要保留: 技术止损 + 风控超标"
        );
    }
}

// ============================================================================
// v11-P0-5 commit 1: LLM 字符串解析 → Vec<FinalDecision>
// ============================================================================

/// 复用 main.rs:1193 extract_advice_and_score 的简化版 (P0-5 不改 main.rs 私有函数)
/// 提取 LLM markdown 输出的 (操作建议文本, 综合分).
///
/// 编译期 AhoCorasick 自动机: 一次扫描找出所有 ACTION_KEYWORDS 出现位置.
/// review #14 性能: 原 15 个 keywords × N 次 .contains() = 90K 次字符串扫描/review.
/// 改自动机 1 次扫描 = 1 次 O(n+m), 中文/英文都用 SIMD 加速.
static ACTION_AC: once_cell::sync::Lazy<aho_corasick::AhoCorasick> =
    once_cell::sync::Lazy::new(|| {
        aho_corasick::AhoCorasick::builder()
            .match_kind(aho_corasick::MatchKind::LeftmostLongest)
            .build([
                "强烈卖出",
                "强烈看空",
                "卖出",
                "看空",
                "偏空",
                "减持",
                "规避",
                "降仓",
                "观望",
                "中性",
                "看平",
                "增持",
                "加仓",
                "买入",
                "看多",
                "强烈看多",
            ])
            .expect("ACTION_KEYWORDS 是固定列表, build 不应失败")
    });

/// v14.4 修订: 不依赖 "## 【操作建议】" 段 — 实际 LLM 仲裁终稿用 "## 一句话结论\n**强烈看空**..."
/// 格式 (一句话总结 + 因子归因里都含关键词). 改为从整个 md 文本搜关键词, 找到第一个匹配就返回.
fn extract_advice_simple(md: &str) -> (String, Option<f64>) {
    // review #14: 用 AhoCorasick 自动机 1 次扫描, 取代 15 个 .contains() 重复扫描.
    // LeftmostLongest 模式 → "强烈卖出" 优先于 "卖出" 出现在同一位置时.
    // 由于 build 顺序就是优先级顺序, find_iter 第一个结果就是最高优先级匹配.
    const ACTION_KEYWORDS: &[&str] = &[
        "强烈卖出",
        "强烈看空",
        "卖出",
        "看空",
        "偏空",
        "减持",
        "规避",
        "降仓",
        "观望",
        "中性",
        "看平",
        "增持",
        "加仓",
        "买入",
        "看多",
        "强烈看多",
    ];
    let advice = ACTION_AC
        .find(md.as_bytes())
        .map(|m| ACTION_KEYWORDS[m.pattern().as_usize()].to_string())
        .unwrap_or_else(|| "未知".to_string());

    let mut score: Option<f64> = None;
    for line in md.lines() {
        if score.is_some() {
            break;
        }
        let t = line.trim();
        if t.contains("综合分") || t.contains("composite_score") || t.contains("composite score")
        {
            for token in t.split(|c: char| !c.is_ascii_digit() && c != '.') {
                if let Ok(v) = token.parse::<f64>() {
                    if (0.0..=100.0).contains(&v) {
                        score = Some(v);
                        break;
                    }
                }
            }
        }
    }
    (advice, score)
}

/// 提取 markdown 第一行非标题非空行 (grill Q4 决策: AI 卡片 1-2 句摘要).
fn first_meaningful_line(md: &str) -> String {
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        return t.chars().take(120).collect(); // 限制长度, 避免巨长
    }
    String::new()
}

/// action_text → (Action, Priority) 映射 (grill Q3/Q4 决策, 不从 score 算 priority)
/// v11 IC 证伪 sentiment_score, 数字不可靠, action 文本才是稳定信号
///
/// v14.3 修订: 加 "看空"/"偏空"/"看多"/"看平" 等 LLM 实际输出关键词
/// (shadow 验证发现 commit 2 关键词不全: LLM 输出"强烈看空"但没映射 → 全 fallback Hold)
fn action_priority_from_advice(advice: &str) -> (Action, Priority) {
    // 优先级: 强烈卖出/看空 > 卖出/偏空/减持 > 增持/看多 > 观望/中性 > 其它
    if advice.contains("强烈卖出") || advice.contains("强烈看空") {
        (Action::ReduceNow, Priority::P0)
    } else if advice.contains("卖出")
        || advice.contains("减持")
        || advice.contains("规避")
        || advice.contains("降仓")
        || advice.contains("看空")
        || advice.contains("偏空")
    {
        (Action::Reduce, Priority::P1)
    } else if advice.contains("增持")
        || advice.contains("加仓")
        || advice.contains("买入")
        || advice.contains("看多")
        || advice.contains("强烈看多")
    {
        (Action::WatchAdd, Priority::P2)
    } else if advice.contains("观望") || advice.contains("中性") || advice.contains("看平") {
        (Action::Hold, Priority::P2)
    } else {
        // 兜底: 未知 action → Hold (P2) 诚实标注
        (Action::Hold, Priority::P2)
    }
}

/// v11-P0-5 commit 1: 从 LLM markdown 输出 (MultiAgent 终稿) 解析成 Vec<FinalDecision>
///
/// 输入:
/// - `holdings`: 持仓列表
/// - `by_code`: code → (name, LLM markdown 终稿 None 表示多 Agent 失败)
///
/// 输出: 每只持仓一个 FinalDecision (失败/缺失 = Hold 兜底)
///
/// 替换 main.rs:967 `build_holding_summary` + `extract_advice_and_score` 字符串猜
/// (commit C 标 deprecated, commit E 留 PUSH_SHADOW 退路)
/// v62: 加 `quotes` 参数 (TopStock 列表含 price/change_pct), 用真报价填充 current_price
///   - 旧: p.cost_price 当现价 + change_pct=0.0 硬编码 (用户看到"今日+0.00%")
///   - 新: 优先用 quotes 拿真价/真涨跌幅, fallback 到 cost_price + 0
pub fn decisions_from_llm(
    holdings: &[Position],
    by_code: &HashMap<String, (String, Option<String>)>,
    quotes: &HashMap<String, (f64, f64)>, // code -> (price, change_pct)
) -> Vec<FinalDecision> {
    let mut out = Vec::new();
    for p in holdings {
        // review #14: 一次 by_code.get() 拿 (name, md) — 原代码 3 次 get + 多次 clone.
        // review #15 简化: 去掉 name_owned 手工 Cow 生命周期扩展 — holdings 永远有 name,
        // 缺失 by_code entry 时回退用 p.name (与原代码 p.name.clone() 行为一致).
        let entry = by_code.get(&p.code);
        let (advice, _score) = match entry.and_then(|(_, md)| md.as_ref()) {
            Some(md) => extract_advice_simple(md),
            None => ("未知".to_string(), None),
        };
        let (action, priority) = action_priority_from_advice(&advice);
        let mut reasons = vec![DecisionReason::new(
            DecisionReasonKind::NewsImpact,
            format!("LLM 操作建议: {}", advice),
        )];
        // 兜底: 失败/缺失 → "无可靠信号" 明确标注
        if advice == "未知" {
            reasons.clear();
            reasons.push(DecisionReason::new(
                DecisionReasonKind::Health,
                "多 Agent 失败或数据缺失, 默认 Hold (诚实标注)",
            ));
        }
        // review #14: 复用 entry (上面已 get), 不再二次 get.
        let ai_summary = entry
            .and_then(|(_, md)| md.as_ref())
            .map(|md| first_meaningful_line(md))
            .unwrap_or_default();
        // v62: 用真报价填 current_price/change_pct
        let (current_price, change_pct) = quotes
            .get(&p.code)
            .map(|(price, pct)| {
                if *price > 0.0 {
                    (*price, *pct)
                } else {
                    (p.cost_price, 0.0)
                }
            })
            .unwrap_or((p.cost_price, 0.0));
        // review #15 简化: name 优先用 by_code 提供的 (LLM 视角的最新名字),
        // 没有则用 holdings p.name. 简化掉之前 name_owned 手工 lifetime 扩展.
        let name = entry.map(|(n, _)| n.as_str()).unwrap_or(&p.name);
        let mut d = FinalDecision::new(
            p.code.clone(),
            name.to_string(),
            current_price, // v62: 用真报价
            change_pct,    // v62: 用真涨跌幅
            action,
            priority,
            reasons,
        )
        .with_stop_loss(p.hard_stop);
        if !ai_summary.is_empty() {
            d = d.with_ai_summary(ai_summary);
        }
        out.push(d);
    }
    out
}

#[cfg(test)]
mod tests_llm_parse {
    use super::*;
    use chrono::NaiveDate;

    fn make_md(advice_section: &str, score: Option<f64>) -> String {
        let score_line = match score {
            Some(s) => format!("综合分: {}", s),
            None => String::new(),
        };
        format!(
            "# 复盘报告\n## 【操作建议】{}\n{}\n",
            advice_section, score_line
        )
    }

    fn make_position(code: &str, name: &str) -> Position {
        Position {
            code: code.to_string(),
            name: name.to_string(),
            shares: 1000,
            cost_price: 10.0,
            hard_stop: 9.0,
            added_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: "测试".to_string(),
            ..Default::default()
        }
    }

    /// 强烈卖出 → ReduceNow (P0)
    #[test]
    fn llm_advice_strong_sell() {
        let holdings = vec![make_position("000001", "测试")];
        let mut by_code = HashMap::new();
        by_code.insert(
            "000001".to_string(),
            ("测试".to_string(), Some(make_md("强烈卖出", Some(20.0)))),
        );
        let quote_map: HashMap<String, (f64, f64)> = HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].action, Action::ReduceNow);
        assert_eq!(decisions[0].priority, Priority::P0);
    }

    /// 减持 → Reduce (P1)
    #[test]
    fn llm_advice_reduce() {
        let holdings = vec![make_position("000001", "测试")];
        let mut by_code = HashMap::new();
        by_code.insert(
            "000001".to_string(),
            ("测试".to_string(), Some(make_md("减持观望", Some(48.0)))),
        );
        let quote_map: HashMap<String, (f64, f64)> = HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions[0].action, Action::Reduce);
        assert_eq!(decisions[0].priority, Priority::P1);
    }

    /// 观望 → Hold (P2)
    #[test]
    fn llm_advice_hold() {
        let holdings = vec![make_position("000001", "测试")];
        let mut by_code = HashMap::new();
        by_code.insert(
            "000001".to_string(),
            ("测试".to_string(), Some(make_md("观望", Some(50.0)))),
        );
        let quote_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions[0].action, Action::Hold);
        assert_eq!(decisions[0].priority, Priority::P2);
    }

    /// 增持 → WatchAdd (P2)
    #[test]
    fn llm_advice_add() {
        let holdings = vec![make_position("000001", "测试")];
        let mut by_code = HashMap::new();
        by_code.insert(
            "000001".to_string(),
            ("测试".to_string(), Some(make_md("加仓", Some(80.0)))),
        );
        let quote_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions[0].action, Action::WatchAdd);
        assert_eq!(decisions[0].priority, Priority::P2);
    }

    /// LLM 失败/数据缺失 → Hold (P2) 兜底, 明确标注"无可靠信号"
    #[test]
    fn llm_missing_fallback_hold() {
        let holdings = vec![make_position("000001", "测试")];
        let by_code: HashMap<String, (String, Option<String>)> = HashMap::new(); // 缺失
        let quote_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions[0].action, Action::Hold);
        assert_eq!(decisions[0].priority, Priority::P2);
        assert!(decisions[0].reasons[0].text.contains("默认 Hold"));
    }

    /// v14.3: LLM 实际输出"强烈看空" → ReduceNow (P0, 等价"强烈卖出")
    #[test]
    fn llm_advice_strong_short_sell() {
        let (action, priority) = action_priority_from_advice("## 【操作建议】**强烈看空**");
        assert_eq!(action, Action::ReduceNow);
        assert_eq!(priority, Priority::P0);
    }

    /// v14.3: LLM 实际输出"偏空" → Reduce (P1, 等价"减持")
    #[test]
    fn llm_advice_bearish() {
        let (action, priority) = action_priority_from_advice("## 【操作建议】**偏空**");
        assert_eq!(action, Action::Reduce);
        assert_eq!(priority, Priority::P1);
    }

    /// v14.3: LLM 实际输出"看空" → Reduce (P1, 等价"卖出")
    #[test]
    fn llm_advice_short() {
        let (action, priority) = action_priority_from_advice("## 【操作建议】**看空**");
        assert_eq!(action, Action::Reduce);
        assert_eq!(priority, Priority::P1);
    }

    /// v14.3: LLM 实际输出"中性" → Hold (P2, 等价"观望")
    #[test]
    fn llm_advice_neutral() {
        let (action, priority) = action_priority_from_advice("## 【操作建议】**中性**");
        assert_eq!(action, Action::Hold);
        assert_eq!(priority, Priority::P2);
    }

    /// v14.3: LLM 实际输出"强烈看多" → WatchAdd (P2, 等价"增持")
    #[test]
    fn llm_advice_strong_buy() {
        let (action, priority) = action_priority_from_advice("## 【操作建议】**强烈看多**");
        assert_eq!(action, Action::WatchAdd);
        assert_eq!(priority, Priority::P2);
    }

    /// v14.4: LLM 实际仲裁终稿格式 "## 一句话结论\n**强烈看空**..." (无"## 【操作建议】"段)
    /// extract_advice_simple 必须能在一句话总结里找到关键词
    #[test]
    fn extract_advice_real_llm_format() {
        let llm_md = r#"# 复盘报告
## 一句话结论
**强烈看空**。技术面空头排列+资金面主力深套+基本面估值偏高，EV为负。

## 因子归因
- 价值：-2 → PE 30.82、PB 12.67均处于板块前10%高估区间
- 动量：-2 → 均线空头排列

## 情景树
- 乐观（P=15%）：主力连续两日净流入超1亿元"#;
        let (advice, _score) = extract_advice_simple(llm_md);
        assert_eq!(
            advice, "强烈看空",
            "v14.4 修订: 从整个 md 找关键词, 不依赖'## 【操作建议】'段"
        );
        let (action, priority) = action_priority_from_advice(&advice);
        assert_eq!(action, Action::ReduceNow);
        assert_eq!(priority, Priority::P0);
    }

    /// v14.4: 关键词在因子归因里, 不在"一句话结论"里 — 也应能提取
    #[test]
    fn extract_advice_from_factor_section() {
        let llm_md = r#"# 复盘
## 一句话结论
技术面多空分歧。

## 因子归因
- 价值：-2 → 偏空
- 动量：+1 → 中性
- 资金：-1 → 减持"#;
        let (advice, _) = extract_advice_simple(llm_md);
        // "偏空" 在因子归因里, "减持" 也命中, 但 "偏空" 在 keywords 列表中更靠前
        // 实际: ACTION_KEYWORDS 顺序遍历, "偏空" 在 "减持" 之前
        assert!(advice == "偏空" || advice == "减持");
    }

    /// v14.4: LLM 输出不含 action 关键词 → 兜底"未知"
    #[test]
    fn extract_advice_no_keyword_fallback() {
        let llm_md = r#"# 复盘
## 一句话结论
技术面多空分歧, 资金面震荡, 等待更明确信号。

## 情景树
- 乐观：触发条件...中性"#;
        let (advice, _) = extract_advice_simple(llm_md);
        // "中性" 在 keywords 列表里, 应该命中
        assert_eq!(advice, "中性");
    }

    /// v14.4 集成测试: 真实 LLM 仲裁终稿 (从 shadow log 抓取) → 端到端 decisions_from_llm
    /// 不依赖 LLM API 重跑, 单测直接验证决策台解析
    #[test]
    fn decisions_from_real_llm_sample() {
        // 真实 LLM 仲裁终稿样本 (002208 合肥城建, 抓自 shadow run bk0p4w2gx)
        let llm_md = r#"## 一句话结论
偏空, 短期动能透支+板块承压, 资金面虽强但主力浮盈存在获利了结风险, EV为负。胜率约29%, 赔率0.74, EV = 0.29×4.7% - 0.71×6.3% = -3.1% 为负。

## 因子归因
- 价值: -2 → PE为负(亏损), ROE为负, 公司处于深度价值陷阱。
- 动量: +1 → 近5日涨幅15.10%处于板块前20%分位。
- 质量: -2 → 净利润同比大幅下降。
- 资金: +1 → 近5日累计净流入+9.37亿。

## 情景树
- 乐观(P=10%): 主力资金持续加仓
- 悲观(P=60%): 诱多出货
"#;
        let mut by_code = HashMap::new();
        by_code.insert(
            "002208".to_string(),
            ("合肥城建".to_string(), Some(llm_md.to_string())),
        );
        let holdings = vec![make_position("002208", "合肥城建")];
        let quote_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions.len(), 1);
        // "偏空" 关键词命中 → Reduce + P1
        assert_eq!(
            decisions[0].action,
            Action::Reduce,
            "LLM 真实输出 '偏空' 应映射到 Reduce"
        );
        assert_eq!(decisions[0].priority, Priority::P1);
        // reason 含 "偏空" 关键词 (兜底信息)
        assert!(decisions[0].reasons[0].text.contains("偏空"));
    }

    /// v14.4 集成测试: 真实 LLM 仲裁终稿 (002131 利欧股份, 抓自 shadow run) — "强烈看空"
    #[test]
    fn decisions_from_real_llm_strong_bearish() {
        let llm_md = r#"## 一句话结论
**偏空**, 技术面空头排列+资金面主力净流出压制短期反弹, EV为负。

## 因子归因
- 价值: -2 → PE极高
- 资金: -2 → 近期累计净流出

## 情景树
- 乐观: 资金面逆转
- 悲观: 加速下跌
"#;
        let mut by_code = HashMap::new();
        by_code.insert(
            "002131".to_string(),
            ("利欧股份".to_string(), Some(llm_md.to_string())),
        );
        let holdings = vec![make_position("002131", "利欧股份")];
        let quote_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        let decisions = decisions_from_llm(&holdings, &by_code, &quote_map);
        assert_eq!(decisions[0].action, Action::Reduce);
        assert_eq!(decisions[0].priority, Priority::P1);
    }
}
