// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - AI分析层
//! ===================================
//!
//! 职责：
//! 1. 封装 Gemini/OpenAI/豆包 API 调用逻辑
//! 2. 利用搜索服务获取实时新闻
//! 3. 结合技术面和消息面生成分析报告
//!
//! 支持的AI模型（按优先级）：
//! - 豆包 (Doubao) - 字节跳动AI模型
//! - OpenAI - GPT系列模型
//! - Gemini - Google AI模型

mod analyze;
mod client;
mod macro_rec;
mod prompts;
mod types;

pub use types::{AnalysisResult, GeminiConfig};

use std::cell::RefCell;
use std::collections::HashMap;

use lazy_static::lazy_static;
use log::{debug, info};
use serde_json::Value;

// ============================================================================
// 常量和股票名称映射
// ============================================================================

lazy_static! {
    /// 股票名称映射（常见股票）
    pub(super) static ref STOCK_NAME_MAP: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("600519", "贵州茅台");
        m.insert("000858", "五粮液");
        m.insert("601318", "中国平安");
        m.insert("600036", "招商银行");
        m.insert("000001", "平安银行");
        m.insert("600000", "浦发银行");
        m.insert("601398", "工商银行");
        m.insert("601288", "农业银行");
        m.insert("601988", "中国银行");
        m.insert("601939", "建设银行");
        m
    };
}

/// Gemini AI 分析器
///
/// 职责：
/// 1. 调用 Google Gemini API、OpenAI 兼容 API 或豆包 API 进行股票分析
/// 2. 结合预先搜索的新闻和技术面数据生成分析报告
/// 3. 解析 AI 返回的 JSON 格式结果
#[derive(Clone)]
pub struct GeminiAnalyzer {
    pub(super) config: GeminiConfig,
    pub(super) client: reqwest::Client,
    pub(super) current_model: RefCell<String>,
    pub(super) using_fallback: RefCell<bool>,
    pub(super) use_openai: bool,
    pub(super) use_doubao: bool,
}

impl GeminiAnalyzer {
    /// 系统提示词 - 决策仪表盘 v2.0
    pub(super) const SYSTEM_PROMPT: &'static str = r#"你是一位专注于趋势交易的 A 股投资分析师，负责生成专业的【决策仪表盘】分析报告。

## 核心交易理念（必须严格遵守）

### 1. 严进策略（乖离率）
- **公式**：(现价 - MA5) / MA5 × 100%
- 乖离率 < 2%：最佳买点区间
- 乖离率 2-5%：可小仓介入
- 乖离率 > 8%：严禁追高！直接判定为"观望"

### 2. 趋势交易（顺势而为）
- **多头排列必须条件**：MA5 > MA10 > MA20
- 只做多头排列的股票，空头排列坚决不碰
- 均线发散上行优于均线粘合
- MACD 辅助判断：DIFF 上穿 DEA 金叉为买点，死叉为卖点
- RSI 辅助判断：<30 超卖可能反弹，>70 超买警惕回调，>80 严禁追高

### 3. 主力资金动向（代理指标）
由于无直接资金流数据，使用以下代理指标推断主力动向：
- **放量上涨（量比>1.5 + pct_chg>0）**：主力介入迹象
- **放量下跌（量比>1.5 + pct_chg<0）**：主力出货迹象
- **缩量上涨**：惜售，但需警惕乏力
- **高换手 + 横盘**：主力筹码交换，关注突破方向

### 3.1 筹码分布研判（若 prompt 含【筹码分布】片段则必须纳入）
- **获利盘比例**：>85% 高位风险（警惕获利回吐）；60-85% 趋势健康；30-60% 上方套牢盘压力大；<30% 深套弱势
- **筹码集中度（90%成本区间宽度/均价）**：<15% 高度集中（主力锁仓/底部磨底，突破即爆发）；15-25% 较集中；>40% 分散（多空分歧大）
- **当前价 vs 主力成本**：
  - 现价高于主力成本 5%+ → 主力浮盈，上涨空间取决于获利盘抛压
  - 现价贴近主力成本（±2%）→ 关键支撑/压力位，突破/破位信号明确
  - 现价低于主力成本 8%+ → 主力深套，反弹遇成本线易受阻，慎追高
- **单峰密集 + 低集中度宽度** → 洗盘完成、筹码交换充分，突破概率高
- **获利盘快速从 <30% 抬升至 >60%** + 放量 → 主力成本抬升、拉升中继

### 4. 买点偏好（回踩支撑）
- **最佳买点**：缩量回踩 MA5 获得支撑
- **次优买点**：回踩 MA10 获得支撑
- **观望情况**：跌破 MA20 时观望

### 5. 涨停板特殊处理
- **首板涨停（涨幅 9.8-10%）**：判定为"强势"但非追高时机，次日低开可关注
- **大涨（>5% 但非涨停）**：短期强势但警惕乖离率扩大
- **连板/N板**：情绪推动，风险陡增，operation_advice 倾向"观望"
- **创业板/科创板涨停 20%**：波动剧烈，仓位控制更严

### 6. 板块联动分析
- 如宏观消息面提及本股所属板块，权重应提升（trend_prediction 加强）
- 如消息面仅提及其他板块而本股板块缺席，警惕跟涨乏力
- 消息面无相关信息时，单纯技术面研判即可

### 7. 风险排查重点
- 减持公告（股东、高管减持）
- 业绩预亏/大幅下滑
- 监管处罚/立案调查
- 行业政策利空
- 大额解禁
- 地缘政治风险

## 输出格式：决策仪表盘 JSON

请严格按照以下结构输出 JSON，**所有字段必须存在**，不要添加额外字段：

{
  "sentiment_score": 65,
  "trend_prediction": "看多",
  "operation_advice": "买入",
  "confidence_level": "高",
  "trend_analysis": "趋势分析文本",
  "short_term_outlook": "短期展望文本",
  "medium_term_outlook": "中期展望文本",
  "technical_analysis": "技术面综合分析文本",
  "ma_analysis": "均线分析文本",
  "volume_analysis": "量能分析文本（包含主力资金代理判断）",
  "pattern_analysis": "K线形态/MACD/RSI/KDJ 分析",
  "fundamental_analysis": "基本面分析文本",
  "sector_position": "行业地位及板块联动判断",
  "company_highlights": "公司亮点描述",
  "news_summary": "相关新闻摘要",
  "market_sentiment": "市场情绪判断",
  "hot_topics": "相关热点话题",
  "analysis_summary": "分析总结",
  "key_points": "核心要点",
  "risk_warning": "风险提示（必须包含止损位：¥XX.XX元，依据 MA20 或前低）",
  "buy_reason": "买入/不买入理由（若建议买入须包含目标价：¥XX.XX元，依据 52 周高点或压力位）"
}

### 字段约束
- sentiment_score：整数 0-100（>70强烈看多, 60-70看多, 40-60震荡, <40看空）
- trend_prediction：仅限 强烈看多/看多/震荡/看空/强烈看空
- operation_advice：仅限 买入/加仓/持有/减仓/卖出/观望
- confidence_level：仅限 高/中/低
- 其他字段均为字符串，每项 1-3 句话
- **止损位和目标价必须基于数据锚点**：止损位参考 MA20/前低/-8% 三者较高者；目标价参考 52周高点/季度高点/+15% 三者较低者

### sentiment_score 量化评分标准（满分100，按因子加权，不可凭感觉随意给分）
- 均线排列（25分）：多头排列满分，空头排列0分，粘合12分
- 乖离率（20分）：<2%满分，2-5%得12分，>5%得5分，>8%得0分
- 量价配合（15分）：放量上涨或缩量回调满分，放量下跌或缩量上涨5分
- MACD/RSI（10分）：MACD金叉+RSI 40-70满分，死叉/超买超卖0-3分
- 价格位置（10分）：52周低位区满分，中位区6分，高位区2分
- 基本面（10分）：PE<15且PB<2满分，PE<30得7分，PE>30或亏损2分
- 消息面/板块联动（10分）：明确利好+板块共振满分，中性5分，利空0分"#;

    /// 文本分析专用系统提示词（analyze_stock 使用，输出自然语言而非 JSON）
    pub(super) const TEXT_SYSTEM_PROMPT: &'static str = r#"你是一位专注于趋势交易的 A 股投资分析师，擅长结合技术面、基本面、主力资金动向和宏观消息面进行综合研判。

## 核心交易理念（必须严格遵守）

### 1. 严进策略（乖离率）
- 乖离率 < 2%：最佳买点区间
- 乖离率 2-5%：可小仓介入
- 乖离率 > 8%：严禁追高！直接判定为"观望"

### 2. 趋势交易（顺势而为）
- 多头排列必须条件：MA5 > MA10 > MA20
- 只做多头排列的股票，空头排列坚决不碰
- MACD 金叉+RSI 40-70 为健康区间，MACD 死叉或 RSI>80 警惕

### 3. 主力资金动向（代理指标）
- 放量上涨（量比>1.5 + 涨幅为正）→ 主力介入
- 放量下跌 → 主力出货
- 高换手 + 横盘 → 筹码交换，关注突破

### 3.1 筹码分布（若上下文含【筹码分布】片段则必须研判）
- 获利盘 >85% 警惕高位抛压；<30% 深套慎追高
- 90%成本区间宽度 <15% 表示筹码高度集中，主力锁仓
- 现价贴近主力成本 → 关键支撑/压力位；现价远低于主力成本 → 主力深套勿抄底

### 4. 涨停板特殊策略
- 首板涨停：强势但非追高点，次日低开可关注
- 连板/N板：情绪推动，仓位严控，倾向观望
- 大涨但未涨停（>5%）：警惕乖离率扩大

### 5. 板块联动
- 如宏观消息涉及本股板块 → 做多倾向加强
- 板块缺席但个股异动 → 警惕跟涨乏力

### 6. 买点偏好（回踩支撑）
- 最佳买点：缩量回踩 MA5 获得支撑
- 次优买点：回踩 MA10 获得支撑
- 观望情况：跌破 MA20 时观望

### 7. 风险排查重点
- 减持公告、业绩预亏、监管处罚、行业政策利空、大额解禁、地缘政治风险

## 输出要求
- 使用中文，结构清晰，分段输出
- 每个部分不超过 3 句话，重点突出
- **必须给出止损位（¥X.XX 元）和目标价（¥X.XX 元）**
- 止损位参考 MA20/前低/-8% 三者较高者
- 目标价参考 52周高点/季度高点/+15% 三者较低者
- 不要输出 JSON，直接输出分析文本"#;

    /// 创建新的分析器
    pub fn new(config: GeminiConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let current_model = config.model_name.clone();

        // 判断是否使用豆包 API
        let use_doubao = config.doubao_api_key.is_some();
        // 判断是否使用 OpenAI 兼容 API
        let use_openai = !use_doubao && config.openai_api_key.is_some();

        if use_doubao {
            info!(
                "✓ 使用豆包 API: {} ({})",
                config.doubao_model,
                config
                    .doubao_base_url
                    .as_deref()
                    .unwrap_or("https://ark.cn-beijing.volces.com/api/v3")
            );
        } else if use_openai {
            info!(
                "✓ 使用 OpenAI 兼容 API: {} ({})",
                config.openai_model,
                config.openai_base_url.as_deref().unwrap_or("官方 API")
            );
        } else {
            info!("✓ 使用 Gemini API: {}", current_model);
        }

        Self {
            config,
            client,
            current_model: RefCell::new(current_model),
            using_fallback: RefCell::new(false),
            use_openai,
            use_doubao,
        }
    }

    /// 从环境变量创建配置
    pub fn from_env() -> Self {
        let api_key = std::env::var("GEMINI_API_KEY").ok();
        let model_name =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_string());
        let fallback_model = std::env::var("GEMINI_FALLBACK_MODEL")
            .unwrap_or_else(|_| "gemini-2.5-flash-lite".to_string());

        let max_retries = std::env::var("GEMINI_MAX_RETRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);

        let retry_delay = std::env::var("GEMINI_RETRY_DELAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2.0);

        let request_delay = std::env::var("GEMINI_REQUEST_DELAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        // OpenAI 兼容 API 配置
        let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
        let openai_base_url = std::env::var("OPENAI_BASE_URL").ok();
        let openai_model =
            std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

        // 豆包 API 配置
        let doubao_api_key = std::env::var("DOUBAO_API_KEY").ok();
        let doubao_base_url = std::env::var("DOUBAO_BASE_URL")
            .ok()
            .or_else(|| Some("https://ark.cn-beijing.volces.com/api/v3".to_string()));
        let doubao_model = std::env::var("DOUBAO_MODEL")
            .unwrap_or_else(|_| "ep-20241230184254-j6pvd".to_string());

        let config = GeminiConfig {
            api_key,
            model_name,
            fallback_model,
            max_retries,
            retry_delay,
            request_delay,
            openai_api_key,
            openai_base_url,
            openai_model,
            doubao_api_key,
            doubao_base_url,
            doubao_model,
        };

        Self::new(config)
    }

    /// 检查分析器是否可用
    pub fn is_available(&self) -> bool {
        self.config.api_key.is_some()
            || self.config.openai_api_key.is_some()
            || self.config.doubao_api_key.is_some()
    }

    /// 获取股票名称
    pub(super) fn get_stock_name(&self, context: &HashMap<String, Value>, code: &str) -> String {
        // 优先使用 context 中的 name
        if let Some(name) = context.get("name").and_then(|v| v.as_str()) {
            if !name.is_empty() {
                return name.to_string();
            }
        }

        // 从映射表查找
        if let Some(name) = STOCK_NAME_MAP.get(code) {
            return name.to_string();
        }

        // 默认返回代码
        format!("股票{}", code)
    }

    /// 切换到备选模型
    pub(super) fn switch_to_fallback(&self) {
        let mut using_fallback = self.using_fallback.borrow_mut();
        if !*using_fallback {
            let fallback = self.config.fallback_model.clone();
            debug!(
                "🔄 切换到备选模型: {} -> {}",
                self.current_model.borrow(),
                fallback
            );
            *self.current_model.borrow_mut() = fallback;
            *using_fallback = true;
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

pub(super) fn get_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub(super) fn format_volume(volume: Option<f64>) -> String {
    match volume {
        Some(v) if v >= 1e8 => format!("{:.2} 亿股", v / 1e8),
        Some(v) if v >= 1e4 => format!("{:.2} 万股", v / 1e4),
        Some(v) => format!("{:.0} 股", v),
        None => "N/A".to_string(),
    }
}

// ============================================================================
// 单例
// ============================================================================

use once_cell::sync::OnceCell;

static ANALYZER: OnceCell<std::sync::Mutex<GeminiAnalyzer>> = OnceCell::new();

/// 获取分析器单例
pub fn get_analyzer() -> &'static std::sync::Mutex<GeminiAnalyzer> {
    ANALYZER.get_or_init(|| std::sync::Mutex::new(GeminiAnalyzer::from_env()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_analyzer_creation() {
        let config = GeminiConfig::default();
        let analyzer = GeminiAnalyzer::new(config);
        // 没有配置 API Key，应该不可用
        assert!(!analyzer.is_available());
    }
}
