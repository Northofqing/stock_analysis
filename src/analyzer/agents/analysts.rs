//! 4 个领域分析师 Agent。每个 Agent **只看自己领域的数据切片**，输出结构化 JSON。

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::analyzer::types::AgentMode;
use crate::analyzer::GeminiAnalyzer;

use super::slices::DomainSlices;

/// 单个分析师的结构化输出（仲裁可数值化聚合）
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct AnalystView {
    /// 0-100 本领域评分（>=70 强多, 60-70 偏多, 40-60 中性, <=40 偏空）
    #[serde(default)]
    pub score: i32,
    /// bull / bear / neutral
    #[serde(default)]
    pub stance: String,
    /// high / medium / low
    #[serde(default)]
    pub confidence: String,
    /// 3-5 条关键信号
    #[serde(default)]
    pub key_signals: Vec<String>,
    /// 1-2 句总结
    #[serde(default)]
    pub summary: String,
}

impl AnalystView {
    pub fn empty(reason: &str) -> Self {
        Self {
            score: 50,
            stance: "neutral".to_string(),
            confidence: "low".to_string(),
            key_signals: vec![],
            summary: format!("（{}）", reason),
        }
    }

    /// 转为 markdown 片段（供下游 Agent 阅读）
    pub fn to_markdown(&self, role: &str) -> String {
        let signals = if self.key_signals.is_empty() {
            "（无关键信号）".to_string()
        } else {
            self.key_signals
                .iter()
                .map(|s| format!("  - {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "### {} — score={} stance={} confidence={}\n摘要：{}\n关键信号：\n{}",
            role,
            self.score,
            empty_or(&self.stance, "neutral"),
            empty_or(&self.confidence, "low"),
            empty_or(&self.summary, "（无）"),
            signals
        )
    }
}

fn empty_or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.trim().is_empty() {
        fallback
    } else {
        s
    }
}

const JSON_FORMAT_NOTE: &str = r#"
## 输出格式（必须是合法 JSON，不要包含其他文字、不要 markdown 围栏）
{
  "score": 0-100整数,
  "stance": "bull" | "bear" | "neutral",
  "confidence": "high" | "medium" | "low",
  "key_signals": ["信号1", "信号2", ...],
  "summary": "1-2 句中文总结"
}
"#;

const FUNDAMENTAL_SYS: &str = r#"你是 A 股**基本面分析师**。只看估值与财务数据，不要分析技术面/资金面/消息面。

评分规则（score 0-100，从基础分 50 开始加减）：
- PE<15 且 PB<3 加 25 分；PE 15-30 加 12 分；PE>30 或亏损 -15 分
- ROE>20% 加 15 分；15-20% 加 8 分；<5% -8 分
- 毛利率>40% 加 10 分；20-40% 加 5 分
- 营收/净利润同比>30% 加 15 分；10-30% 加 8 分；<0 -10 分
- 总分裁剪到 0-100

stance：score>=60 → bull；score<=40 → bear；其余 neutral。
confidence：数据完整度高且信号一致 → high；缺数据/分歧大 → low。"#;

const TECHNICAL_SYS: &str = r#"你是 A 股**技术面分析师**。只看均线/乖离/MACD/RSI/KDJ/价格区间/近期走势/涨跌停，不要分析基本面/资金面/消息面。

评分规则（score 0-100，从基础分 50 开始加减）：
- 多头排列 +20；空头排列 -20；粘合 0
- MA5乖离率：<2% +15；2-5% +5；5-8% -10；>8% -25
- MACD 金叉 +10；死叉 -10
- RSI 30-70 区间 +5；<30 +3（超卖反弹）；>80 -15（严禁追高）
- KDJ 多头 +3；超买区 -8
- 52周位置：<30% +8；>80% -8
- 涨跌停：连板 -15；首板 -5；大涨>5%但乖离>5% -10
- **布林+MACD 共振信号（强约束，若上下文有【布林+MACD 共振信号】段落则必须遵循）**：
  - 上轨减仓 TopSell（顶背离/红柱缩短/死叉） -25，stance 必须 bear
  - 主升浪启动 UptrendStart（布林张口+0轴上方金叉） +20
  - 下轨抄底 BottomBuy（底背离/0轴下绿柱缩短） +12
  - 准备变盘 PreReversal（布林收口） +0（仅提示）
- 总分裁剪到 0-100

stance：score>=60 → bull；score<=40 → bear；其余 neutral。"#;

const CAPITAL_SYS: &str = r#"你是 A 股**资金面分析师**。只看量能/主力代理/真实主力资金流/分时/龙虎榜/筹码分布，不要分析基本面/技术指标/新闻。

评分规则（score 0-100，从基础分 50 开始加减）：
- 真实主力净流入+股价涨 +20；近3日累计净流入 +10
- 真实主力净流出+股价涨 -20（诱多/出货警惕）
- 真实主力净流出+股价跌 -10
- 放量上涨 +12；放量下跌 -15；缩量回踩均线 +8
- 龙虎榜：知名游资/机构净买入 +10；机构净卖出 -10
- 筹码集中度<15% 且现价贴近主力成本 +8
- 获利盘>85% -8；<30% -5
- 总分裁剪到 0-100

若无真实资金/龙虎榜/筹码数据，仅用代理判断，confidence=low。
stance：score>=60 → bull；score<=40 → bear；其余 neutral。"#;

const NEWS_SYS: &str = r#"你是 A 股**消息面分析师**。只看新闻舆情、宏观事件、板块联动、政策与情绪，不要分析价格/财务/资金。

评分规则（score 0-100，从基础分 50 开始加减）：
- 重大利好（业绩超预期/中标/重组/政策点名本股板块）+25
- 一般利好（行业景气/同板块上涨/分析师上调）+12
- 一般利空（行业政策收紧/同板块下跌）-12
- 重大利空（业绩预亏/减持/监管处罚/解禁）-25
- 板块联动：宏观点名所属板块 +8；只点其他板块（跟涨乏力）-5
- 总分裁剪到 0-100

若新闻数据不足，confidence=low，score 默认 50。
stance：score>=60 → bull；score<=40 → bear；其余 neutral。"#;

const SECTOR_SYS: &str = r#"你是 A 股**行业/板块联动分析师**。只评估"标的所属板块当前是否在风口、与大盘/同板块联动情况"，不要分析公司财务/技术指标/资金流。

评分规则（score 0-100，从基础分 50 开始加减）：
- 宏观/新闻明确点名"本股所属板块"+ 利好政策/景气度上行 +25
- 同板块龙头/相邻板块今日大涨（强联动） +15
- 板块在国家战略级方向（半导体/新能源/AI/军工/医药/高端制造等）+10
- 名称/代码无法识别明确板块或板块成分股较杂 -5（confidence=low）
- 板块出现政策收紧/集体下跌/被监管点名 -25
- 公司是板块尾部小盘股，仅"跟涨乏力"型联动 -8
- 总分裁剪到 0-100

若数据不足以判断板块归属（无新闻/宏观提及）：score=50，confidence=low，stance=neutral。
key_signals 必须包含一条形如"所属板块：XXX"的判断与依据。"#;

const TIMEFRAME_SYS: &str = r#"你是 A 股**时间窗口分析师**。基于技术切片，**同时**评估短期（1-3 交易日）与中期（2-4 周）的可行性，并给出统一打分。

评分规则（score 0-100，从基础分 50 开始加减）：
- 短期：近 5 日累计涨幅 0~5% +8；>10% -10（透支）；<-10% +5（超跌反弹概率）
- 短期：MA5乖离>5% -10；MA5乖离<2% +5
- 短期：MACD 刚金叉 +10；刚死叉 -10
- 中期：多头排列（MA5>10>20）+15；空头排列 -15
- 中期：价格站稳 MA20 上方 +8；跌破 MA20 -8
- 中期：52周位置 <40% +8；>80% -10
- 总分裁剪到 0-100

key_signals 必须分别给出"短期(1-3日)：xxx"和"中期(2-4周)：xxx"两条以上判断。
summary 一句话归纳"短期与中期方向是否一致"。
stance：score>=60 → bull；score<=40 → bear；其余 neutral。"#;

fn build_prompt(role: &str, slice: &str, basics: &str) -> String {
    format!(
        "# {} 任务\n\n## 标的\n{}\n## 本领域数据\n{}\n{}",
        role, basics, slice, JSON_FORMAT_NOTE
    )
}

async fn run_analyst(
    analyzer: &GeminiAnalyzer,
    sys: &str,
    role: &str,
    slice: &str,
    basics: &str,
) -> Result<AnalystView> {
    let prompt = build_prompt(role, slice, basics);
    let raw = analyzer
        .call_api_mode(&prompt, sys, AgentMode::Quick)
        .await?;
    parse_view(&raw, role)
}

fn parse_view(raw: &str, role: &str) -> Result<AnalystView> {
    let cleaned = raw.replace("```json", "").replace("```", "");
    let start = cleaned.find('{');
    let end = cleaned.rfind('}');
    if let (Some(s), Some(e)) = (start, end) {
        if e > s {
            let json_str = &cleaned[s..=e];
            match serde_json::from_str::<AnalystView>(json_str) {
                Ok(mut v) => {
                    v.score = v.score.clamp(0, 100);
                    if v.stance.is_empty() {
                        v.stance = if v.score >= 60 {
                            "bull".to_string()
                        } else if v.score <= 40 {
                            "bear".to_string()
                        } else {
                            "neutral".to_string()
                        };
                    }
                    if v.confidence.is_empty() {
                        v.confidence = "medium".to_string();
                    }
                    return Ok(v);
                }
                Err(e) => {
                    return Err(anyhow!("[{}] JSON 解析失败: {}", role, e));
                }
            }
        }
    }
    Err(anyhow!("[{}] 响应未包含 JSON 对象", role))
}

pub(super) async fn run_fundamental(
    analyzer: &GeminiAnalyzer,
    slices: &DomainSlices,
) -> AnalystView {
    run_analyst(
        analyzer,
        FUNDAMENTAL_SYS,
        "基本面分析师",
        &slices.fundamental,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[基本面 Agent] {}", e);
        AnalystView::empty("基本面 Agent 失败")
    })
}

pub(super) async fn run_technical(analyzer: &GeminiAnalyzer, slices: &DomainSlices) -> AnalystView {
    run_analyst(
        analyzer,
        TECHNICAL_SYS,
        "技术面分析师",
        &slices.technical,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[技术面 Agent] {}", e);
        AnalystView::empty("技术面 Agent 失败")
    })
}

pub(super) async fn run_capital(analyzer: &GeminiAnalyzer, slices: &DomainSlices) -> AnalystView {
    run_analyst(
        analyzer,
        CAPITAL_SYS,
        "资金面分析师",
        &slices.capital,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[资金面 Agent] {}", e);
        AnalystView::empty("资金面 Agent 失败")
    })
}

pub(super) async fn run_news(analyzer: &GeminiAnalyzer, slices: &DomainSlices) -> AnalystView {
    let news_section = match (&slices.news, &slices.macro_ctx) {
        (Some(n), Some(m)) => format!("【新闻舆情】\n{}\n\n【宏观背景】\n{}", n, m),
        (Some(n), None) => format!("【新闻舆情】\n{}", n),
        (None, Some(m)) => format!("【宏观背景】\n{}", m),
        (None, None) => return AnalystView::empty("无新闻/宏观数据"),
    };
    run_analyst(
        analyzer,
        NEWS_SYS,
        "消息面分析师",
        &news_section,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[消息面 Agent] {}", e);
        AnalystView::empty("消息面 Agent 失败")
    })
}

pub(super) async fn run_sector(analyzer: &GeminiAnalyzer, slices: &DomainSlices) -> AnalystView {
    run_analyst(
        analyzer,
        SECTOR_SYS,
        "行业板块分析师",
        &slices.sector,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[行业板块 Agent] {}", e);
        AnalystView::empty("行业板块 Agent 失败")
    })
}

pub(super) async fn run_timeframe(analyzer: &GeminiAnalyzer, slices: &DomainSlices) -> AnalystView {
    run_analyst(
        analyzer,
        TIMEFRAME_SYS,
        "时间窗口分析师",
        &slices.technical,
        &slices.basics,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!("[时间窗口 Agent] {}", e);
        AnalystView::empty("时间窗口 Agent 失败")
    })
}
