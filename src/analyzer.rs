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

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// 常量和股票名称映射
// ============================================================================

lazy_static! {
    /// 股票名称映射（常见股票）
    static ref STOCK_NAME_MAP: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("600519", "贵州茅台");
        m.insert("000001", "平安银行");
        m.insert("300750", "宁德时代");
        m.insert("002594", "比亚迪");
        m.insert("600036", "招商银行");
        m.insert("601318", "中国平安");
        m.insert("000858", "五粮液");
        m.insert("600276", "恒瑞医药");
        m.insert("601012", "隆基绿能");
        m.insert("002475", "立讯精密");
        m.insert("300059", "东方财富");
        m.insert("002415", "海康威视");
        m.insert("600900", "长江电力");
        m.insert("601166", "兴业银行");
        m.insert("600028", "中国石化");
        m
    };
}

// ============================================================================
// 数据结构
// ============================================================================

/// AI 分析结果数据类 - 决策仪表盘版
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub code: String,
    pub name: String,

    // ========== 核心指标 ==========
    /// 综合评分 0-100 (>70强烈看多, >60看多, 40-60震荡, <40看空)
    pub sentiment_score: i32,
    /// 趋势预测：强烈看多/看多/震荡/看空/强烈看空
    pub trend_prediction: String,
    /// 操作建议：买入/加仓/持有/减仓/卖出/观望
    pub operation_advice: String,
    /// 置信度：高/中/低
    pub confidence_level: String,

    // ========== 决策仪表盘 ==========
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<Value>,

    // ========== 走势分析 ==========
    #[serde(default)]
    pub trend_analysis: String,
    #[serde(default)]
    pub short_term_outlook: String,
    #[serde(default)]
    pub medium_term_outlook: String,

    // ========== 技术面分析 ==========
    #[serde(default)]
    pub technical_analysis: String,
    #[serde(default)]
    pub ma_analysis: String,
    #[serde(default)]
    pub volume_analysis: String,
    #[serde(default)]
    pub pattern_analysis: String,

    // ========== 基本面分析 ==========
    #[serde(default)]
    pub fundamental_analysis: String,
    #[serde(default)]
    pub sector_position: String,
    #[serde(default)]
    pub company_highlights: String,

    // ========== 情绪面/消息面分析 ==========
    #[serde(default)]
    pub news_summary: String,
    #[serde(default)]
    pub market_sentiment: String,
    #[serde(default)]
    pub hot_topics: String,

    // ========== 综合分析 ==========
    #[serde(default)]
    pub analysis_summary: String,
    #[serde(default)]
    pub key_points: String,
    #[serde(default)]
    pub risk_warning: String,
    #[serde(default)]
    pub buy_reason: String,

    // ========== 元数据 ==========
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
    pub search_performed: bool,
    #[serde(default)]
    pub data_sources: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl AnalysisResult {
    /// 获取核心结论（一句话）
    pub fn get_core_conclusion(&self) -> String {
        if let Some(dashboard) = &self.dashboard {
            if let Some(core) = dashboard.get("core_conclusion") {
                if let Some(sentence) = core.get("one_sentence") {
                    if let Some(s) = sentence.as_str() {
                        return s.to_string();
                    }
                }
            }
        }
        self.analysis_summary.clone()
    }

    /// 获取持仓建议
    pub fn get_position_advice(&self, has_position: bool) -> String {
        if let Some(dashboard) = &self.dashboard {
            if let Some(core) = dashboard.get("core_conclusion") {
                if let Some(advice) = core.get("position_advice") {
                    let key = if has_position { "has_position" } else { "no_position" };
                    if let Some(val) = advice.get(key) {
                        if let Some(s) = val.as_str() {
                            return s.to_string();
                        }
                    }
                }
            }
        }
        self.operation_advice.clone()
    }

    /// 根据操作建议返回对应 emoji
    pub fn get_emoji(&self) -> &'static str {
        match self.operation_advice.as_str() {
            "买入" | "加仓" => "🟢",
            "强烈买入" => "💚",
            "持有" => "🟡",
            "观望" => "⚪",
            "减仓" => "🟠",
            "卖出" => "🔴",
            "强烈卖出" => "❌",
            _ => "🟡",
        }
    }

    /// 返回置信度星级
    pub fn get_confidence_stars(&self) -> &'static str {
        match self.confidence_level.as_str() {
            "高" => "⭐⭐⭐",
            "中" => "⭐⭐",
            "低" => "⭐",
            _ => "⭐⭐",
        }
    }
}

// ============================================================================
// GeminiAnalyzer 主结构
// ============================================================================

/// Gemini AI 分析器配置
#[derive(Debug, Clone)]
pub struct GeminiConfig {
    /// Gemini API Key
    pub api_key: Option<String>,
    /// 主模型名称
    pub model_name: String,
    /// 备选模型名称
    pub fallback_model: String,
    /// 最大重试次数
    pub max_retries: usize,
    /// 重试基础延迟（秒）
    pub retry_delay: f64,
    /// 请求前延迟（秒）
    pub request_delay: f64,
    /// OpenAI 兼容 API 配置
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: String,
    /// 豆包 (Doubao) API 配置
    pub doubao_api_key: Option<String>,
    pub doubao_base_url: Option<String>,
    pub doubao_model: String,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model_name: "gemini-2.0-flash-exp".to_string(),
            fallback_model: "gemini-1.5-flash".to_string(),
            max_retries: 3,
            retry_delay: 5.0,
            request_delay: 1.0,
            openai_api_key: None,
            openai_base_url: None,
            openai_model: "gpt-4".to_string(),
            doubao_api_key: None,
            doubao_base_url: Some("https://ark.cn-beijing.volces.com/api/v3".to_string()),
            doubao_model: "ep-20241230184254-j6pvd".to_string(),
        }
    }
}

/// Gemini AI 分析器
///
/// 职责：
/// 1. 调用 Google Gemini API、OpenAI 兼容 API 或豆包 API 进行股票分析
/// 2. 结合预先搜索的新闻和技术面数据生成分析报告
/// 3. 解析 AI 返回的 JSON 格式结果
#[derive(Clone)]
pub struct GeminiAnalyzer {
    config: GeminiConfig,
    client: reqwest::Client,
    current_model: RefCell<String>,
    using_fallback: RefCell<bool>,
    use_openai: bool,
    use_doubao: bool,
}

impl GeminiAnalyzer {
    /// 系统提示词 - 决策仪表盘 v2.0
    const SYSTEM_PROMPT: &'static str = r#"你是一位专注于趋势交易的 A 股投资分析师，负责生成专业的【决策仪表盘】分析报告。

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
    const TEXT_SYSTEM_PROMPT: &'static str = r#"你是一位专注于趋势交易的 A 股投资分析师，擅长结合技术面、基本面、主力资金动向和宏观消息面进行综合研判。

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
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        let current_model = config.model_name.clone();
        
        // 检查是否使用豆包（优先级最高）
        let use_doubao = config.doubao_api_key.is_some() 
            && config.api_key.is_none();
        
        // 检查是否使用 OpenAI（优先级次之）
        let use_openai = config.openai_api_key.is_some() 
            && config.api_key.is_none()
            && !use_doubao;

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
        let mut config = GeminiConfig::default();

        // 读取 Gemini 配置
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            if !key.starts_with("your_") && key.len() > 10 {
                config.api_key = Some(key);
            }
        }

        if let Ok(model) = std::env::var("GEMINI_MODEL") {
            config.model_name = model;
        }

        if let Ok(fallback) = std::env::var("GEMINI_MODEL_FALLBACK") {
            config.fallback_model = fallback;
        }

        // 读取 OpenAI 配置
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.starts_with("your_") && key.len() > 10 {
                config.openai_api_key = Some(key);
            }
        }

        if let Ok(base_url) = std::env::var("OPENAI_BASE_URL") {
            if base_url.starts_with("http") {
                config.openai_base_url = Some(base_url);
            }
        }

        if let Ok(model) = std::env::var("OPENAI_MODEL") {
            config.openai_model = model;
        }

        // 读取豆包配置
        if let Ok(key) = std::env::var("DOUBAO_API_KEY") {
            if !key.starts_with("your_") && key.len() > 10 {
                config.doubao_api_key = Some(key);
            }
        }

        if let Ok(base_url) = std::env::var("DOUBAO_BASE_URL") {
            if base_url.starts_with("http") {
                config.doubao_base_url = Some(base_url);
            }
        }

        if let Ok(model) = std::env::var("DOUBAO_MODEL") {
            config.doubao_model = model;
        }

        Self::new(config)
    }

    /// 检查分析器是否可用
    pub fn is_available(&self) -> bool {
        self.config.api_key.is_some() 
            || self.config.openai_api_key.is_some() 
            || self.config.doubao_api_key.is_some()
    }

    /// 简化的股票分析方法（用于pipeline）
    pub async fn analyze_stock(
        &self,
        code: &str,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
    ) -> Result<String> {
        if kline_data.is_empty() {
            return Err(anyhow!("数据为空"));
        }

        // 构建简化的分析上下文
        let latest = &kline_data[0];
        
        // 基础行情数据
        let mut context = format!(
            "股票代码: {}\n\
            最新价: {:.2}\n\
            开盘: {:.2}\n\
            最高: {:.2}\n\
            最低: {:.2}\n\
            成交量: {:.0}\n\
            成交额: {:.0}\n\
            涨跌幅: {:.2}%\n",
            code,
            latest.close,
            latest.open,
            latest.high,
            latest.low,
            latest.volume,
            latest.amount,
            latest.pct_chg
        );

        // ========== 均线系统与乖离率（从历史K线计算） ==========
        let closes: Vec<f64> = kline_data.iter().map(|k| k.close).collect();
        let data_len = closes.len();

        let calc_ma = |period: usize| -> Option<f64> {
            if data_len >= period {
                Some(closes[..period].iter().sum::<f64>() / period as f64)
            } else {
                None
            }
        };

        let ma5 = calc_ma(5);
        let ma10 = calc_ma(10);
        let ma20 = calc_ma(20);
        let ma60 = calc_ma(60);

        context.push_str("\n【均线系统】\n");
        if let Some(v) = ma5 { context.push_str(&format!("MA5: {:.2}\n", v)); }
        if let Some(v) = ma10 { context.push_str(&format!("MA10: {:.2}\n", v)); }
        if let Some(v) = ma20 { context.push_str(&format!("MA20: {:.2}\n", v)); }
        if let Some(v) = ma60 { context.push_str(&format!("MA60: {:.2}\n", v)); }

        // 多头/空头排列判断
        if let (Some(m5), Some(m10), Some(m20)) = (ma5, ma10, ma20) {
            let alignment = if m5 > m10 && m10 > m20 {
                "多头排列 ✅ (MA5>MA10>MA20)"
            } else if m5 < m10 && m10 < m20 {
                "空头排列 ❌ (MA5<MA10<MA20)"
            } else {
                "均线粘合/交叉，趋势不明"
            };
            context.push_str(&format!("均线排列: {}\n", alignment));
        }

        // 乖离率
        if let Some(m5) = ma5 {
            if m5 > 0.0 {
                let bias5 = (latest.close - m5) / m5 * 100.0;
                let bias_warning = if bias5.abs() > 5.0 { "⚠️ 偏离过大" }
                    else if bias5.abs() > 2.0 { "注意回归" }
                    else { "正常范围" };
                context.push_str(&format!("MA5乖离率: {:.2}% ({})\n", bias5, bias_warning));
            }
        }
        if let Some(m10) = ma10 {
            if m10 > 0.0 {
                let bias10 = (latest.close - m10) / m10 * 100.0;
                context.push_str(&format!("MA10乖离率: {:.2}%\n", bias10));
            }
        }
        if let Some(m20) = ma20 {
            if m20 > 0.0 {
                let bias20 = (latest.close - m20) / m20 * 100.0;
                context.push_str(&format!("MA20乖离率: {:.2}%\n", bias20));
            }
        }

        // ========== 量能分析 ==========
        context.push_str("\n【量能分析】\n");
        if data_len >= 5 {
            let vol_5d_avg = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
            if vol_5d_avg > 0.0 {
                let volume_ratio = latest.volume / vol_5d_avg;
                let vol_status = if volume_ratio > 2.0 { "显著放量" }
                    else if volume_ratio > 1.2 { "温和放量" }
                    else if volume_ratio > 0.8 { "量能平稳" }
                    else { "明显缩量" };
                context.push_str(&format!("5日量比: {:.2} ({})\n", volume_ratio, vol_status));
            }
        }
        if data_len >= 10 {
            let vol_10d_avg = kline_data[..10].iter().map(|k| k.volume).sum::<f64>() / 10.0;
            if vol_10d_avg > 0.0 {
                let volume_ratio_10 = latest.volume / vol_10d_avg;
                context.push_str(&format!("10日量比: {:.2}\n", volume_ratio_10));
            }
        }

        // ========== 主力资金动向（代理指标） ==========
        context.push_str("\n【主力资金动向（代理）】\n");
        if data_len >= 5 {
            let vol_5d_avg = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
            let vol_ratio = if vol_5d_avg > 0.0 { latest.volume / vol_5d_avg } else { 1.0 };
            let money_flow = if vol_ratio > 1.5 && latest.pct_chg > 1.0 {
                "🔥 放量上涨 — 主力介入迹象"
            } else if vol_ratio > 1.5 && latest.pct_chg < -1.0 {
                "⚠️ 放量下跌 — 主力出货迹象"
            } else if vol_ratio < 0.7 && latest.pct_chg > 0.5 {
                "缩量上涨 — 惜售但动能不足"
            } else if vol_ratio < 0.7 && latest.pct_chg < -0.5 {
                "缩量下跌 — 抛压减弱"
            } else if vol_ratio > 1.3 && latest.pct_chg.abs() < 1.0 {
                "高换手+横盘 — 筹码交换，关注突破方向"
            } else {
                "量价关系平稳，无明显主力动向"
            };
            context.push_str(&format!("代理判断: {}\n", money_flow));
            if let Some(turnover) = latest.turnover_rate {
                context.push_str(&format!("换手率: {:.2}%（>7%活跃，>15%火热）\n", turnover));
            }
        }

        // ========== 涨跌停识别 ==========
        let is_gem = code.starts_with("300") || code.starts_with("301"); // 创业板
        let is_star = code.starts_with("688"); // 科创板
        let is_bj = code.starts_with("8") || code.starts_with("9") || code.starts_with("4"); // 北交所
        // ST 股通过股票名称判断（若可用），此处保守不考虑 ST
        let limit_pct = if is_gem || is_star { 20.0 }
            else if is_bj { 30.0 }
            else { 10.0 };
        if latest.pct_chg >= limit_pct - 0.3 {
            context.push_str(&format!("\n【涨跌停】🚀 今日涨停（涨幅 {:.2}% / 涨停阈值 {}%）— ", latest.pct_chg, limit_pct));
            // 检查连板数
            let mut consec = 1;
            for k in kline_data[1..].iter().take(10) {
                if k.pct_chg >= limit_pct - 0.3 { consec += 1; } else { break; }
            }
            if consec >= 2 {
                context.push_str(&format!("连续 {} 板！情绪推动风险陡增，建议观望\n", consec));
            } else {
                context.push_str("首板，非追高时机，次日低开可关注\n");
            }
        } else if latest.pct_chg <= -(limit_pct - 0.3) {
            context.push_str(&format!("\n【涨跌停】📉 今日跌停（跌幅 {:.2}%）— 承压严重，规避\n", latest.pct_chg));
        } else if latest.pct_chg >= 5.0 {
            context.push_str(&format!("\n【涨跌停】📈 大涨 {:.2}%（未涨停）— 短期强势，警惕乖离扩大\n", latest.pct_chg));
        }

        // ========== MACD (12,26,9) ==========
        if data_len >= 26 {
            // 注意：kline_data[0] 是最新，计算 EMA 需要按时间顺序（旧→新）
            let mut chron: Vec<f64> = closes.iter().rev().copied().collect();
            // 仅取最近 60 根以提升效率（足够让 EMA 收敛）
            if chron.len() > 120 { chron = chron[chron.len()-120..].to_vec(); }
            let ema = |period: usize, src: &[f64]| -> Vec<f64> {
                let alpha = 2.0 / (period as f64 + 1.0);
                let mut out = Vec::with_capacity(src.len());
                let mut prev = src[0];
                out.push(prev);
                for &v in &src[1..] {
                    prev = alpha * v + (1.0 - alpha) * prev;
                    out.push(prev);
                }
                out
            };
            let ema12 = ema(12, &chron);
            let ema26 = ema(26, &chron);
            let diff: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a,b)| a-b).collect();
            let dea = ema(9, &diff);
            let n = diff.len();
            let macd = 2.0 * (diff[n-1] - dea[n-1]);
            let macd_signal = if diff[n-1] > dea[n-1] && n >= 2 && diff[n-2] <= dea[n-2] {
                "金叉 ✅"
            } else if diff[n-1] < dea[n-1] && n >= 2 && diff[n-2] >= dea[n-2] {
                "死叉 ❌"
            } else if diff[n-1] > dea[n-1] {
                "多头区间"
            } else {
                "空头区间"
            };
            context.push_str(&format!(
                "\n【MACD】DIFF: {:.3}, DEA: {:.3}, MACD柱: {:.3} ({})\n",
                diff[n-1], dea[n-1], macd, macd_signal
            ));
        }

        // ========== RSI(14) - Wilder 平滑 ==========
        if data_len >= 15 {
            let chron: Vec<f64> = closes.iter().rev().copied().collect();
            let mut gains = 0.0;
            let mut losses = 0.0;
            for i in 1..=14 {
                let diff = chron[i] - chron[i-1];
                if diff > 0.0 { gains += diff; } else { losses -= diff; }
            }
            let mut avg_gain = gains / 14.0;
            let mut avg_loss = losses / 14.0;
            for i in 15..chron.len() {
                let diff = chron[i] - chron[i-1];
                let (g, l) = if diff > 0.0 { (diff, 0.0) } else { (0.0, -diff) };
                avg_gain = (avg_gain * 13.0 + g) / 14.0;
                avg_loss = (avg_loss * 13.0 + l) / 14.0;
            }
            let rsi = if avg_loss.abs() < 1e-9 { 100.0 } else {
                100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
            };
            let rsi_signal = if rsi > 80.0 { "严重超买 🔴" }
                else if rsi > 70.0 { "超买" }
                else if rsi < 20.0 { "严重超卖 🟢" }
                else if rsi < 30.0 { "超卖" }
                else { "正常" };
            context.push_str(&format!("【RSI(14)】{:.2} ({})\n", rsi, rsi_signal));
        }

        // ========== KDJ(9,3,3) ==========
        if data_len >= 9 {
            let chron: Vec<&crate::data_provider::KlineData> = kline_data.iter().rev().collect();
            let mut k_val = 50.0;
            let mut d_val = 50.0;
            let n = chron.len();
            let start = n.saturating_sub(30).max(8); // 让 KDJ 迭代收敛
            for i in start..n {
                let window_start = i.saturating_sub(8);
                let window = &chron[window_start..=i];
                let hh = window.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
                let ll = window.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
                let rsv = if (hh - ll).abs() < 1e-9 { 50.0 }
                    else { (chron[i].close - ll) / (hh - ll) * 100.0 };
                k_val = 2.0 / 3.0 * k_val + 1.0 / 3.0 * rsv;
                d_val = 2.0 / 3.0 * d_val + 1.0 / 3.0 * k_val;
            }
            let j_val = 3.0 * k_val - 2.0 * d_val;
            let kdj_signal = if k_val > 80.0 && d_val > 80.0 { "超买区" }
                else if k_val < 20.0 && d_val < 20.0 { "超卖区" }
                else if k_val > d_val { "多头" }
                else { "空头" };
            context.push_str(&format!(
                "【KDJ】K: {:.2}, D: {:.2}, J: {:.2} ({})\n",
                k_val, d_val, j_val, kdj_signal
            ));
        }

        // ========== 52周（约250个交易日）高低价 ==========
        context.push_str("\n【价格区间指标】\n");
        let week52_len = data_len.min(250);
        if week52_len >= 5 {
            let week52_data = &kline_data[..week52_len];
            let high_52w = week52_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_52w = week52_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_in_range = if (high_52w - low_52w).abs() > 0.001 {
                (latest.close - low_52w) / (high_52w - low_52w) * 100.0
            } else {
                50.0
            };
            context.push_str(&format!(
                "52周最高: {:.2} | 52周最低: {:.2}\n\
                当前价位于52周区间: {:.1}% (0%=最低, 100%=最高)\n",
                high_52w, low_52w, pos_in_range
            ));
        }

        // 一季度（约60个交易日）高低价
        let quarter_len = data_len.min(60);
        if quarter_len >= 5 {
            let quarter_data = &kline_data[..quarter_len];
            let high_q = quarter_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_q = quarter_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_q = if (high_q - low_q).abs() > 0.001 {
                (latest.close - low_q) / (high_q - low_q) * 100.0
            } else {
                50.0
            };
            context.push_str(&format!(
                "近一季最高: {:.2} | 近一季最低: {:.2}\n\
                当前价位于季度区间: {:.1}%\n",
                high_q, low_q, pos_q
            ));
        }

        // ========== 近期走势明细（最近10个交易日） ==========
        let recent_len = data_len.min(10);
        if recent_len >= 2 {
            context.push_str("\n【近期走势】\n");
            context.push_str("日期 | 收盘价 | 涨跌幅 | 成交量\n");
            for k in kline_data[..recent_len].iter() {
                context.push_str(&format!(
                    "{} | {:.2} | {:.2}% | {:.0}\n",
                    k.date, k.close, k.pct_chg, k.volume
                ));
            }

            // 近5日/10日累计涨幅
            let chg_5d: f64 = kline_data[..data_len.min(5)].iter().map(|k| k.pct_chg).sum();
            context.push_str(&format!("近5日累计涨幅: {:.2}%\n", chg_5d));
            if recent_len >= 10 {
                let chg_10d: f64 = kline_data[..10].iter().map(|k| k.pct_chg).sum();
                context.push_str(&format!("近10日累计涨幅: {:.2}%\n", chg_10d));
            }

            // 波动率（近期日收益标准差）
            let returns: Vec<f64> = kline_data[..recent_len].iter().map(|k| k.pct_chg).collect();
            let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / returns.len() as f64;
            let volatility = variance.sqrt();
            context.push_str(&format!("近期日波动率: {:.2}%\n", volatility));
        }

        // ========== 盈利指标（估值+财务） ==========
        if latest.pe_ratio.is_some() || latest.pb_ratio.is_some() || latest.market_cap.is_some() {
            context.push_str("\n【盈利水平指标】\n");
            
            // 估值指标
            if let Some(pe) = latest.pe_ratio {
                let pe_level = if pe < 0.0 { "亏损" }
                    else if pe < 15.0 { "估值合理" }
                    else if pe < 30.0 { "估值适中" }
                    else { "估值偏高" };
                context.push_str(&format!("市盈率(PE): {:.2} ({})\n", pe, pe_level));
            }
            
            if let Some(pb) = latest.pb_ratio {
                let pb_level = if pb < 1.0 { "可能被低估" }
                    else if pb < 3.0 { "市净率正常" }
                    else { "市净率较高" };
                context.push_str(&format!("市净率(PB): {:.2} ({})\n", pb, pb_level));
            }
            
            // 市值规模与流通性
            if let Some(market_cap) = latest.market_cap {
                let cap_type = if market_cap < 50.0 { "小盘股" }
                    else if market_cap < 300.0 { "中盘股" }
                    else if market_cap < 1000.0 { "大盘股" }
                    else { "超大盘股" };
                context.push_str(&format!("总市值: {:.2}亿元 ({})\n", market_cap, cap_type));
                
                if let Some(circ_cap) = latest.circulating_cap {
                    let circulation_ratio = (circ_cap / market_cap) * 100.0;
                    let liquidity = if circulation_ratio < 30.0 { "低流通，控盘严密" }
                        else if circulation_ratio < 70.0 { "中等流通" }
                        else { "高流通，交易自由" };
                    context.push_str(&format!("流通市值: {:.2}亿元 (流通比例: {:.1}%, {})\n", 
                        circ_cap, circulation_ratio, liquidity));
                }
            }
            
            // 交易活跃度
            if let Some(turnover) = latest.turnover_rate {
                let activity = if turnover < 1.0 { "极度清淡，关注度低" }
                    else if turnover < 3.0 { "交投清淡" }
                    else if turnover < 7.0 { "换手正常" }
                    else if turnover < 15.0 { "交易活跃" }
                    else { "换手火热，资金关注度高" };
                context.push_str(&format!("换手率: {:.2}% ({})\n", turnover, activity));
            }
            
            // 估值综合评估
            if let (Some(pe), Some(pb)) = (latest.pe_ratio, latest.pb_ratio) {
                if pe > 0.0 {
                    let pe_pb_ratio = pe / pb.max(0.1);
                    let valuation = if pe_pb_ratio < 3.0 && pe < 20.0 && pb < 2.0 {
                        "整体估值较低，具有投资价值"
                    } else if pe_pb_ratio < 5.0 && pe < 30.0 {
                        "估值适中"
                    } else {
                        "估值偏高，需谨慎"
                    };
                    context.push_str(&format!("估值综合评估: {}\n", valuation));
                }
            }
        }

        // ========== 财务指标（盈利能力+成长性） ==========
        let has_financials = latest.eps.is_some() || latest.roe.is_some()
            || latest.gross_margin.is_some() || latest.revenue_yoy.is_some();
        if has_financials {
            context.push_str("\n【财务指标】\n");

            if let Some(eps) = latest.eps {
                let eps_assessment = if eps < 0.0 { "亏损" }
                    else if eps < 0.5 { "盈利较弱" }
                    else if eps < 2.0 { "盈利正常" }
                    else { "盈利优秀" };
                context.push_str(&format!("每股收益(EPS): {:.3}元 ({})\n", eps, eps_assessment));
            }

            if let Some(roe) = latest.roe {
                let roe_assessment = if roe < 5.0 { "较低" }
                    else if roe < 15.0 { "正常" }
                    else if roe < 25.0 { "优秀" }
                    else { "卓越" };
                context.push_str(&format!("净资产收益率(ROE): {:.2}% ({})\n", roe, roe_assessment));
            }

            if let Some(gm) = latest.gross_margin {
                let gm_assessment = if gm < 20.0 { "竞争激烈" }
                    else if gm < 40.0 { "正常水平" }
                    else if gm < 60.0 { "竞争优势明显" }
                    else { "高壁垒行业" };
                context.push_str(&format!("毛利率: {:.2}% ({})\n", gm, gm_assessment));
            }

            if let Some(nm) = latest.net_margin {
                context.push_str(&format!("净利率: {:.2}%\n", nm));
            }

            if let Some(rev_yoy) = latest.revenue_yoy {
                let growth = if rev_yoy < 0.0 { "营收下滑" }
                    else if rev_yoy < 10.0 { "缓慢增长" }
                    else if rev_yoy < 30.0 { "稳健增长" }
                    else { "高速增长" };
                context.push_str(&format!("营收同比增长: {:.2}% ({})\n", rev_yoy, growth));
            }

            if let Some(profit_yoy) = latest.net_profit_yoy {
                let growth = if profit_yoy < -20.0 { "利润大幅下滑" }
                    else if profit_yoy < 0.0 { "利润下滑" }
                    else if profit_yoy < 20.0 { "利润稳定增长" }
                    else { "利润高速增长" };
                context.push_str(&format!("净利润同比增长: {:.2}% ({})\n", profit_yoy, growth));
            }
        }

        // 夏普比率
        if let Some(sharpe) = latest.sharpe_ratio {
            let sr_assessment = if sharpe < 0.0 { "风险调整后亏损" }
                else if sharpe < 1.0 { "一般" }
                else if sharpe < 2.0 { "良好" }
                else { "优秀" };
            context.push_str(&format!("\n夏普比率: {:.2} ({})\n", sharpe, sr_assessment));
        }

        context.push_str(&format!(
            "\n最近{}天数据点数: {}",
            kline_data.len(),
            kline_data.len()
        ));

        // 宏观市场背景（如有则注入 prompt）
        let macro_section = match macro_context {
            Some(mc) if !mc.is_empty() => format!(
                "\n\n---\n\n## 📡 宏观市场背景（请评估下列最新事件对本股的潜在影响）\n\n{}\n\n---",
                mc
            ),
            _ => String::new(),
        };

        let prompt = format!(
            "请分析以下股票的技术走势和基本面：\n\n{}{}\n\n\
            要求：\n\
            1. 【宏观影响】若有宏观背景信息，先评估国际/政策事件对本股及所属行业的影响；\n\
               - 当前大盘交易环境：牛市/熊市/震荡\n\
               - 情绪面：投资者情绪是偏乐观还是悲观，是否存在恐慌性抛售或过度追高\n\
               - 行业动态：所属行业是否处于景气周期，是否有利好/利空消息\n\
               - 板块联动：若宏观信息提及本股所属板块，强化看多逻辑；若仅提及其他板块，警惕跟涨乏力\n\
               - 地缘政治风险：是否处于敏感时期（如选举、战争、国际冲突等）\n\
            2. 【技术面】分析以下维度：\n\
               - 均线系统：MA5/MA10/MA20排列状态，是否多头/空头排列\n\
               - 乖离率：当前价格偏离MA5的程度，>5%警惕追高风险\n\
               - MACD/RSI/KDJ：金叉死叉、超买超卖信号的综合研判（数据见上下文）\n\
               - 价格位置：当前价位于52周和季度区间的位置，是否接近高点/低点\n\
               - 量价关系：量比变化，放量/缩量配合涨跌的含义\n\
               - 近期走势：近5-10日涨跌趋势和波动率\n\
               - 涨跌停研判：若触及涨跌停或大涨超 5%，按首板/连板/ST/创业板规则区别对待\n\
            3. 【主力资金动向（代理）】基于量价+换手率组合判断：\n\
               - 放量上涨 + 高换手 → 主力介入，看多倾向\n\
               - 放量下跌 → 主力出货，警戒\n\
               - 缩量回踩 MA5/MA10 → 理想买点\n\
               - 高换手 + 横盘 → 筹码交换，关注突破方向\n\
            4. 【基本面】如果有盈利指标，请重点分析：\n\
               - 估值水平：PE、PB是否合理（PE<15优秀，15-30正常，>30偏高；PB<1低估，1-3正常，>3偏高）\n\
               - 盈利能力：EPS、ROE、毛利率、净利率反映的公司竞争力\n\
               - 成长性：营收和净利润同比增长率判断成长阶段\n\
               - 市值规模：小盘股成长性强但风险高，大盘股稳定但弹性小\n\
               - 公司业务亮点：是否有核心竞争力、行业地位、创新能力等\n\
               - 行业地位：在所属行业中的竞争位置\n\
               - 流通性与换手率：流通比例+换手率反映交易活跃度和可能的控盘情况\n\
               - 估值综合：结合PE、PB、EPS判断当前价格是否合理\n\
               - 夏普比率：风险调整后收益水平\n\
            5. 【操作建议与价位】基于技术面+资金面+基本面+消息面给出明确的操作建议：\n\
               - 操作：买入/加仓/持有/减仓/卖出/观望（乖离率>5%不追高，空头排列不做多，连板观望）\n\
               - **必须给出具体数字**：建议买入价 ¥X.XX 元，目标价（止盈）¥X.XX 元，止损位 ¥X.XX 元\n\
               - 止损位参考：MA20/近期前低/-8% 三者较高者；目标价参考：52周高点/季度高点/+15% 三者较低者\n\
            6. 【风险提示】指出主要风险因素（估值风险、技术风险、流动性风险、\
               52周高点压力、波动率异常、宏观风险、板块轮动风险等）\n\
            \n请简明扼要，重点突出，每个部分不超过3句话。",
            context,
            macro_section
        );

        // 调用API（使用文本分析专用系统提示词）
        self.call_api_with_retry_ex(&prompt, Self::TEXT_SYSTEM_PROMPT).await
    }

    /// 分析单只股票
    pub async fn analyze(
        &mut self,
        context: &HashMap<String, Value>,
        news_context: Option<&str>,
    ) -> AnalysisResult {
        let code = context
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        // 获取股票名称
        let name = self.get_stock_name(context, &code);

        // 检查可用性
        if !self.is_available() {
            return AnalysisResult {
                code,
                name,
                sentiment_score: 50,
                trend_prediction: "震荡".to_string(),
                operation_advice: "持有".to_string(),
                confidence_level: "低".to_string(),
                dashboard: None,
                trend_analysis: String::new(),
                short_term_outlook: String::new(),
                medium_term_outlook: String::new(),
                technical_analysis: String::new(),
                ma_analysis: String::new(),
                volume_analysis: String::new(),
                pattern_analysis: String::new(),
                fundamental_analysis: String::new(),
                sector_position: String::new(),
                company_highlights: String::new(),
                news_summary: String::new(),
                market_sentiment: String::new(),
                hot_topics: String::new(),
                analysis_summary: "AI 分析功能未启用（未配置 API Key）".to_string(),
                key_points: String::new(),
                risk_warning: "请配置 API Key 后重试".to_string(),
                buy_reason: String::new(),
                raw_response: None,
                search_performed: false,
                data_sources: String::new(),
                success: false,
                error_message: Some("API Key 未配置".to_string()),
            };
        }

        // 请求前延迟
        if self.config.request_delay > 0.0 {
            debug!(
                "[LLM] 请求前等待 {:.1} 秒...",
                self.config.request_delay
            );
            tokio::time::sleep(Duration::from_secs_f64(self.config.request_delay)).await;
        }

        // 格式化提示词
        let prompt = self.format_prompt(context, &name, news_context);

        info!("========== AI 分析 {}({}) ==========", name, code);
        info!("[LLM配置] 模型: {}", self.current_model.borrow());
        info!("[LLM配置] Prompt 长度: {} 字符", prompt.len());
        info!(
            "[LLM配置] 是否包含新闻: {}",
            if news_context.is_some() { "是" } else { "否" }
        );

        // 调用 API
        let start_time = Instant::now();
        match self.call_api_with_retry(&prompt).await {
            Ok(response_text) => {
                let elapsed = start_time.elapsed().as_secs_f64();
                info!(
                    "[LLM返回] API 响应成功, 耗时 {:.2}s, 响应长度 {} 字符",
                    elapsed,
                    response_text.len()
                );

                // 解析响应
                let mut result = self.parse_response(&response_text, &code, &name);
                result.raw_response = Some(response_text);
                result.search_performed = news_context.is_some();

                info!(
                    "[LLM解析] {}({}) 分析完成: {}, 评分 {}",
                    name, code, result.trend_prediction, result.sentiment_score
                );

                result
            }
            Err(e) => {
                error!("AI 分析 {}({}) 失败: {}", name, code, e);
                AnalysisResult {
                    code,
                    name,
                    sentiment_score: 50,
                    trend_prediction: "震荡".to_string(),
                    operation_advice: "持有".to_string(),
                    confidence_level: "低".to_string(),
                    dashboard: None,
                    trend_analysis: String::new(),
                    short_term_outlook: String::new(),
                    medium_term_outlook: String::new(),
                    technical_analysis: String::new(),
                    ma_analysis: String::new(),
                    volume_analysis: String::new(),
                    pattern_analysis: String::new(),
                    fundamental_analysis: String::new(),
                    sector_position: String::new(),
                    company_highlights: String::new(),
                    news_summary: String::new(),
                    market_sentiment: String::new(),
                    hot_topics: String::new(),
                    analysis_summary: format!("分析过程出错: {}", &e.to_string()[..100.min(e.to_string().len())]),
                    key_points: String::new(),
                    risk_warning: "分析失败，请稍后重试或手动分析".to_string(),
                    buy_reason: String::new(),
                    raw_response: None,
                    search_performed: false,
                    data_sources: String::new(),
                    success: false,
                    error_message: Some(e.to_string()),
                }
            }
        }
    }

    /// 获取股票名称
    fn get_stock_name(&self, context: &HashMap<String, Value>, code: &str) -> String {
        // 优先从上下文获取
        if let Some(name) = context.get("stock_name").and_then(|v| v.as_str()) {
            if !name.starts_with("股票") {
                return name.to_string();
            }
        }

        // 从实时行情获取
        if let Some(realtime) = context.get("realtime") {
            if let Some(name) = realtime.get("name").and_then(|v| v.as_str()) {
                return name.to_string();
            }
        }

        // 从映射表获取
        STOCK_NAME_MAP
            .get(code)
            .map(|&s| s.to_string())
            .unwrap_or_else(|| format!("股票{}", code))
    }

    /// 调用 API（带重试和故障转移，使用默认系统提示词）
    async fn call_api_with_retry(&self, prompt: &str) -> Result<String> {
        self.call_api_with_retry_ex(prompt, Self::SYSTEM_PROMPT).await
    }

    /// 调用 API（带重试和故障转移，自定义系统提示词）
    async fn call_api_with_retry_ex(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        if self.use_doubao {
            return self.call_doubao_api(prompt, system_prompt).await;
        }
        
        if self.use_openai {
            return self.call_openai_api(prompt, system_prompt).await;
        }

        let mut last_error = None;

        for attempt in 0..self.config.max_retries {
            if attempt > 0 {
                let delay = self.config.retry_delay * 2_f64.powi(attempt as i32 - 1);
                let delay = delay.min(60.0);
                info!("[Gemini] 第 {} 次重试，等待 {:.1} 秒...", attempt + 1, delay);
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }

            match self.call_gemini_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                    let error_str = last_error.as_ref().unwrap().to_string();
                    
                    let is_rate_limit = error_str.contains("429") 
                        || error_str.to_lowercase().contains("quota")
                        || error_str.to_lowercase().contains("rate");

                    if is_rate_limit {
                        warn!(
                            "[Gemini] API 限流 (429)，第 {}/{} 次尝试",
                            attempt + 1,
                            self.config.max_retries
                        );

                        // 切换到备选模型
                        if attempt >= self.config.max_retries / 2 && !*self.using_fallback.borrow() {
                            self.switch_to_fallback();
                        }
                    } else {
                        warn!(
                            "[Gemini] API 调用失败，第 {}/{} 次尝试: {}",
                            attempt + 1,
                            self.config.max_retries,
                            &error_str[..100.min(error_str.len())]
                        );
                    }
                }
            }
        }

        // 尝试豆包作为第一备选
        if self.config.doubao_api_key.is_some() {
            warn!("[Gemini] 所有重试失败，切换到豆包 API");
            match self.call_doubao_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(doubao_error) => {
                    error!("[豆包] 备选 API 也失败: {}", doubao_error);
                }
            }
        }

        // 尝试 OpenAI 作为最后的备选
        if self.config.openai_api_key.is_some() {
            warn!("[Gemini] 切换到 OpenAI 兼容 API");
            match self.call_openai_api(prompt, system_prompt).await {
                Ok(response) => return Ok(response),
                Err(openai_error) => {
                    error!("[OpenAI] 备选 API 也失败: {}", openai_error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("所有 AI API 调用失败")))
    }

    /// 调用 Gemini API
    async fn call_gemini_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct GeminiRequest {
            contents: Vec<Content>,
            #[serde(skip_serializing_if = "Option::is_none")]
            system_instruction: Option<SystemInstruction>,
            generation_config: GenerationConfig,
        }

        #[derive(Serialize)]
        struct Content {
            parts: Vec<Part>,
        }

        #[derive(Serialize)]
        struct Part {
            text: String,
        }

        #[derive(Serialize)]
        struct SystemInstruction {
            parts: Vec<Part>,
        }

        #[derive(Serialize)]
        struct GenerationConfig {
            temperature: f32,
            max_output_tokens: u32,
        }

        #[derive(Deserialize)]
        struct GeminiResponse {
            candidates: Vec<Candidate>,
        }

        #[derive(Deserialize)]
        struct Candidate {
            content: ResponseContent,
        }

        #[derive(Deserialize)]
        struct ResponseContent {
            parts: Vec<ResponsePart>,
        }

        #[derive(Deserialize)]
        struct ResponsePart {
            text: String,
        }

        let api_key = self.config.api_key.as_ref().ok_or_else(|| anyhow!("Gemini API Key 未配置"))?;
        
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.current_model.borrow(), api_key
        );

        let request = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part {
                    text: prompt.to_string(),
                }],
            }],
            system_instruction: Some(SystemInstruction {
                parts: vec![Part {
                    text: system_prompt.to_string(),
                }],
            }),
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Gemini API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let gemini_response: GeminiResponse = response.json().await.context("解析 Gemini 响应失败")?;

        gemini_response
            .candidates
            .get(0)
            .and_then(|c| c.content.parts.get(0))
            .map(|p| p.text.clone())
            .ok_or_else(|| anyhow!("Gemini 返回空响应"))
    }

    /// 调用 OpenAI 兼容 API
    async fn call_openai_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct OpenAIRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u32,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }

        #[derive(Deserialize)]
        struct ResponseMessage {
            content: String,
        }

        let api_key = self.config.openai_api_key.as_ref().ok_or_else(|| anyhow!("OpenAI API Key 未配置"))?;
        
        let base_url = self.config.openai_base_url.as_deref().unwrap_or("https://api.openai.com/v1");
        let url = format!("{}/chat/completions", base_url);

        let request = OpenAIRequest {
            model: self.config.openai_model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                },
            ],
            temperature: 0.7,
            max_tokens: 8192,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .context("OpenAI API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let openai_response: OpenAIResponse = response.json().await.context("解析 OpenAI 响应失败")?;

        openai_response
            .choices
            .get(0)
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("OpenAI 返回空响应"))
    }

    /// 调用豆包 (Doubao) API
    async fn call_doubao_api(&self, prompt: &str, system_prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct DoubaoRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u32,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct DoubaoResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }

        #[derive(Deserialize)]
        struct ResponseMessage {
            content: String,
        }

        let api_key = self.config.doubao_api_key.as_ref().ok_or_else(|| anyhow!("豆包 API Key 未配置"))?;
        
        let base_url = self.config.doubao_base_url.as_deref()
            .unwrap_or("https://ark.cn-beijing.volces.com/api/v3");
        let url = format!("{}/chat/completions", base_url);

        let request = DoubaoRequest {
            model: self.config.doubao_model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                },
            ],
            temperature: 0.7,
            max_tokens: 8192,
        };

        info!("[豆包] 调用 API: {} (model: {})", url, self.config.doubao_model);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .context("豆包 API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            error!("[豆包] API 错误: HTTP {}: {}", status, error_text);
            return Err(anyhow!("HTTP {}: {}", status, error_text));
        }

        let doubao_response: DoubaoResponse = response.json().await.context("解析豆包响应失败")?;

        doubao_response
            .choices
            .get(0)
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("豆包返回空响应"))
    }

    /// 切换到备选模型
    fn switch_to_fallback(&self) {
        warn!("[LLM] 切换到备选模型: {}", self.config.fallback_model);
        *self.current_model.borrow_mut() = self.config.fallback_model.clone();
        *self.using_fallback.borrow_mut() = true;
        info!("[LLM] 备选模型 {} 已启用", self.current_model.borrow());
    }

    /// 格式化提示词
    fn format_prompt(
        &self,
        context: &HashMap<String, Value>,
        name: &str,
        news_context: Option<&str>,
    ) -> String {
        let code = context.get("code").and_then(|v| v.as_str()).unwrap_or("Unknown");
        let date = context.get("date").and_then(|v| v.as_str()).unwrap_or("未知");

        let mut prompt = format!(
            r#"# 决策仪表盘分析请求

## 📊 股票基础信息
| 项目 | 数据 |
|------|------|
| 股票代码 | **{}** |
| 股票名称 | **{}** |
| 分析日期 | {} |

---

## 📈 技术面数据
"#,
            code, name, date
        );

        // 添加今日行情
        if let Some(today) = context.get("today") {
            prompt.push_str(&format!(
                r#"
### 今日行情
| 指标 | 数值 |
|------|------|
| 收盘价 | {} 元 |
| 涨跌幅 | {}% |
| 成交量 | {} |
| MA5 | {} |
| MA10 | {} |
| MA20 | {} |
"#,
                today.get("close").and_then(|v| v.as_f64()).unwrap_or(0.0),
                today.get("pct_chg").and_then(|v| v.as_f64()).unwrap_or(0.0),
                format_volume(today.get("volume").and_then(|v| v.as_f64())),
                today.get("ma5").and_then(|v| v.as_f64()).unwrap_or(0.0),
                today.get("ma10").and_then(|v| v.as_f64()).unwrap_or(0.0),
                today.get("ma20").and_then(|v| v.as_f64()).unwrap_or(0.0),
            ));
            
            // 添加盈利指标（基本面数据）
            let has_profitability = today.get("pe_ratio").is_some() 
                || today.get("pb_ratio").is_some()
                || today.get("turnover_rate").is_some()
                || today.get("market_cap").is_some();
                
            if has_profitability {
                prompt.push_str("\n### 盈利水平指标\n| 指标 | 数值 | 评估 |\n|------|------|------|\n");
                
                if let Some(pe) = today.get("pe_ratio").and_then(|v| v.as_f64()) {
                    let pe_assessment = if pe < 0.0 {
                        "亏损"
                    } else if pe < 15.0 {
                        "估值合理 ✅"
                    } else if pe < 30.0 {
                        "估值适中 ⚠️"
                    } else {
                        "估值偏高 🔴"
                    };
                    prompt.push_str(&format!("| 市盈率(PE) | {:.2} | {} |\n", pe, pe_assessment));
                }
                
                if let Some(pb) = today.get("pb_ratio").and_then(|v| v.as_f64()) {
                    let pb_assessment = if pb < 1.0 {
                        "可能被低估 ✅"
                    } else if pb < 3.0 {
                        "市净率正常 ⚠️"
                    } else {
                        "市净率较高 🔴"
                    };
                    prompt.push_str(&format!("| 市净率(PB) | {:.2} | {} |\n", pb, pb_assessment));
                }
                
                if let Some(turnover) = today.get("turnover_rate").and_then(|v| v.as_f64()) {
                    let turnover_assessment = if turnover < 3.0 {
                        "交投清淡"
                    } else if turnover < 10.0 {
                        "正常换手"
                    } else {
                        "活跃交易"
                    };
                    prompt.push_str(&format!("| 换手率 | {:.2}% | {} |\n", turnover, turnover_assessment));
                }
                
                if let Some(market_cap) = today.get("market_cap").and_then(|v| v.as_f64()) {
                    prompt.push_str(&format!("| 总市值 | {:.2}亿元 | - |\n", market_cap));
                }
                
                if let Some(circ_cap) = today.get("circulating_cap").and_then(|v| v.as_f64()) {
                    prompt.push_str(&format!("| 流通市值 | {:.2}亿元 | - |\n", circ_cap));
                }
            }
        }

        // 添加新闻
        prompt.push_str("\n---\n\n## 📰 舆情情报\n");
        if let Some(news) = news_context {
            prompt.push_str(&format!("\n{}\n", news));
        } else {
            prompt.push_str("\n未搜索到该股票近期的相关新闻。请主要依据技术面数据进行分析。\n");
        }

        // 添加分析要求
        prompt.push_str(&format!(
            r#"
---

## ✅ 分析任务

请为 **{}({})** 生成【决策仪表盘】。请输出完整的 JSON，必须包含以下所有字段：
sentiment_score(0-100整数), trend_prediction, operation_advice, confidence_level,
trend_analysis, short_term_outlook, medium_term_outlook,
technical_analysis, ma_analysis, volume_analysis, pattern_analysis,
fundamental_analysis, sector_position, company_highlights,
news_summary, market_sentiment, hot_topics,
analysis_summary, key_points, risk_warning, buy_reason

**关键要求**：
1. sentiment_score 按因子加权评分：均线排列(25)+乖离率(20)+量价配合(15)+MACD/RSI(10)+价格位置(10)+基本面(10)+消息面/板块联动(10)，满分100。
2. volume_analysis 必须包含主力资金动向代理判断（放量上涨/放量下跌/缩量/高换手横盘）。
3. pattern_analysis 必须结合 MACD/RSI/KDJ 信号研判金叉死叉与超买超卖。
4. sector_position 须评估板块联动：如消息面提及本股所属板块则加强看多，否则警惕跟涨乏力。
5. risk_warning 必须包含具体止损位（格式：止损位：¥XX.XX元），参考 MA20/前低/-8% 三者较高者。
6. buy_reason 若建议买入，必须包含具体目标价（格式：目标价：¥XX.XX元），参考 52周高点/季度高点/+15% 三者较低者。
7. 若本股今日涨停或连板，operation_advice 应倾向"观望"；若乖离率>5%，严禁"买入"。

只输出 JSON，不要包含其他文字。
"#,
            name, code
        ));

        prompt
    }

    /// 解析响应
    fn parse_response(&self, response_text: &str, code: &str, name: &str) -> AnalysisResult {
        // 清理响应文本
        let cleaned = response_text
            .replace("```json", "")
            .replace("```", "")
            .trim()
            .to_string();

        // 查找 JSON 内容
        if let Some(json_start) = cleaned.find('{') {
            if let Some(json_end) = cleaned.rfind('}') {
                let json_str = &cleaned[json_start..=json_end];

                // 尝试解析 JSON
                match serde_json::from_str::<Value>(json_str) {
                    Ok(data) => {
                        return AnalysisResult {
                            code: code.to_string(),
                            name: name.to_string(),
                            sentiment_score: data.get("sentiment_score")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(50) as i32,
                            trend_prediction: data.get("trend_prediction")
                                .and_then(|v| v.as_str())
                                .unwrap_or("震荡")
                                .to_string(),
                            operation_advice: data.get("operation_advice")
                                .and_then(|v| v.as_str())
                                .unwrap_or("持有")
                                .to_string(),
                            confidence_level: data.get("confidence_level")
                                .and_then(|v| v.as_str())
                                .unwrap_or("中")
                                .to_string(),
                            dashboard: data.get("dashboard").cloned(),
                            trend_analysis: get_string(&data, "trend_analysis"),
                            short_term_outlook: get_string(&data, "short_term_outlook"),
                            medium_term_outlook: get_string(&data, "medium_term_outlook"),
                            technical_analysis: get_string(&data, "technical_analysis"),
                            ma_analysis: get_string(&data, "ma_analysis"),
                            volume_analysis: get_string(&data, "volume_analysis"),
                            pattern_analysis: get_string(&data, "pattern_analysis"),
                            fundamental_analysis: get_string(&data, "fundamental_analysis"),
                            sector_position: get_string(&data, "sector_position"),
                            company_highlights: get_string(&data, "company_highlights"),
                            news_summary: get_string(&data, "news_summary"),
                            market_sentiment: get_string(&data, "market_sentiment"),
                            hot_topics: get_string(&data, "hot_topics"),
                            analysis_summary: get_string(&data, "analysis_summary"),
                            key_points: get_string(&data, "key_points"),
                            risk_warning: get_string(&data, "risk_warning"),
                            buy_reason: get_string(&data, "buy_reason"),
                            raw_response: None,
                            search_performed: data.get("search_performed")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            data_sources: get_string(&data, "data_sources"),
                            success: true,
                            error_message: None,
                        };
                    }
                    Err(e) => {
                        warn!("JSON 解析失败: {}, 使用文本分析", e);
                    }
                }
            }
        }

        // 文本解析备选方案
        self.parse_text_response(response_text, code, name)
    }

    /// 从纯文本响应中提取分析信息
    fn parse_text_response(&self, response_text: &str, code: &str, name: &str) -> AnalysisResult {
        let text_lower = response_text.to_lowercase();

        let positive_keywords = ["看多", "买入", "上涨", "突破", "强势", "利好", "加仓"];
        let negative_keywords = ["看空", "卖出", "下跌", "跌破", "弱势", "利空", "减仓"];

        let positive_count = positive_keywords.iter().filter(|&&kw| text_lower.contains(kw)).count();
        let negative_count = negative_keywords.iter().filter(|&&kw| text_lower.contains(kw)).count();

        let (sentiment_score, trend, advice) = if positive_count > negative_count + 1 {
            (65, "看多", "买入")
        } else if negative_count > positive_count + 1 {
            (35, "看空", "卖出")
        } else {
            (50, "震荡", "持有")
        };

        let summary = if response_text.len() > 500 {
            &response_text[..500]
        } else {
            response_text
        };

        AnalysisResult {
            code: code.to_string(),
            name: name.to_string(),
            sentiment_score,
            trend_prediction: trend.to_string(),
            operation_advice: advice.to_string(),
            confidence_level: "低".to_string(),
            dashboard: None,
            trend_analysis: String::new(),
            short_term_outlook: String::new(),
            medium_term_outlook: String::new(),
            technical_analysis: String::new(),
            ma_analysis: String::new(),
            volume_analysis: String::new(),
            pattern_analysis: String::new(),
            fundamental_analysis: String::new(),
            sector_position: String::new(),
            company_highlights: String::new(),
            news_summary: String::new(),
            market_sentiment: String::new(),
            hot_topics: String::new(),
            analysis_summary: summary.to_string(),
            key_points: "JSON解析失败，仅供参考".to_string(),
            risk_warning: "分析结果可能不准确，建议结合其他信息判断".to_string(),
            buy_reason: String::new(),
            raw_response: None,
            search_performed: false,
            data_sources: String::new(),
            success: true,
            error_message: None,
        }
    }

    /// 基于宏观新闻，让 AI 推荐当日 A 股受益板块和股票
    pub async fn analyze_macro_recommendations(&self, macro_news: &str) -> Result<String> {
        if !self.is_available() {
            return Err(anyhow!("AI 模型未配置"));
        }
        if macro_news.trim().is_empty() {
            return Err(anyhow!("宏观新闻为空，无法进行推荐"));
        }

        let today = chrono::Local::now().format("%Y年%m月%d日").to_string();

        let prompt = format!(
r#"今天是 {today}。

以下是今日宏观市场最新新闻：
===== 宏观新闻 =====
{macro_news}
===== 新闻结束 =====

请基于上述宏观信息，从 Top-Down 视角进行 **A 股板块和个股推荐**。

直接输出以下 Markdown 结构（不要输出 JSON）：

## 📊 宏观环境解读
（2-3 句话：概括当前宏观核心驱动因素）

## 🔗 产业链推演（Chain-of-Thought）
针对今日最核心的 1-3 条新闻，展示完整推演链路：
- **新闻事件** → 直接受益行业 → 上游供给侧 → 配套基建侧 → 下游需求侧
（示例：AI算力需求爆发 → GPU/服务器 → 电力运营商 → 特高压设备 → 数据中心IDC → AI应用）

## � 实际业务需求分析
从国内外**真实业务需求**出发，分析当前哪些行业处于景气上升期：
**国际需求侧**：
- 全球 AI 基建投资（微软/谷歌/Meta 资本开支）→ 算力硬件出口 → 电力配套需求
- 欧美能源转型/碳中和 → 光伏/风电/储能出口 → 上游硅料/逆变器
- 全球供应链重构（近岸外包）→ 东南亚建厂 → 工程机械/基建设备出口
- 国际油气价格走势 → 油服/管道设备 → 能源安全相关
**国内需求侧**：
- "东数西算" / 数据中心建设 → IDC → 制冷/UPS/光模块 → **电力负荷**
- 新型电力系统建设 → 特高压/柔性直流 → 智能电网/配网设备
- 新能源车渗透率提升 → 充电桩/换电 → 电池回收
- 老旧小区改造/城中村 → 建材/家电 → 物业管理
- 国产替代加速 → 半导体设备/材料/EDA → 信创/操作系统
请结合当前新闻，判断上述哪些需求正在加速，并体现在板块推荐中。

## 🏭 受益板块推荐（Top 3-5 个板块）
对每个板块输出：
- **板块名称**（产业链位置：上游/中游/下游/配套）：受益逻辑（1-2句）
  - 代表性个股：股票代码 + 股票名称（列出 2-3 只）
  - 实际需求验证：该板块对应的真实业务订单/产能/出口数据支撑（1句）
  - 催化剂：近期可能的上涨催化（1句）
  - 风险：主要风险点（1句）

## 🎯 重点关注个股（Top 5-10 只）
每只股票：
| 股票代码 | 股票名称 | 产业链位置 | 逻辑 | 业务需求支撑 | 风险 |

## ⚠️ 今日需回避的板块
（列出 1-3 个宏观不利板块并说明原因）

## 📋 操作建议摘要
（100字以内：总结今日操作策略）

## 📌 推荐代码汇总
请在最后用一行列出所有推荐的 A 股 6 位代码（逗号分隔），格式严格如下：
【推荐代码】600519,000001,300750,002594,601318

**核心要求——产业链思考链路（Chain-of-Thought）**：
不要只看新闻表面提到的行业，必须沿着产业链做**上下游推演**，覆盖直接受益和间接受益板块。

典型推演示例（请举一反三）：
- AI大模型/算力需求爆发 → 算力基础设施(服务器/GPU) → **电力需求激增** → 电力运营(火电/水电/核电) → **特高压/输变电设备** → 电力设备制造商
- 新能源汽车销量增长 → 动力电池 → 上游锂矿/钴矿 → 充电桩/换电站 → 电网改造
- 地缘冲突升级 → 军工/国防 → 上游特种材料/芯片 → 能源安全 → 石油/天然气/煤炭
- 房地产政策放松 → 地产股 → 建材/家居 → 银行(信贷) → 物业管理
- 半导体国产替代 → 设备/材料 → EDA软件 → 封测 → 下游消费电子
每条推荐必须标注属于产业链的哪个环节（上游/中游/下游/配套）。

**宏观规律和市场情绪**：
- 选股第一驱动力是资金追捧的热点题材、连板股方向、龙头股带动效应；真实的订单/产能/出口/政策落地数据作为辅助验证放在情绪之后考虑，有业务支撑的热点更可持续，纯情绪炒作需提示风险
- 国际需求（如全球AI资本开支、欧美碳中和、供应链重构）和国内需求（如东数西算、新型电力系统、国产替代）可作为验证热点持续性的参考框架
- 市场情绪的变化可能对短期股价波动产生显著影响，需结合宏观新闻和产业链分析进行综合判断。

请务必：
1. 代码格式统一用 6 位数字（如 000001、600519），0/3/6 开头
2. 推荐逻辑必须与新闻内容直接相关，不可凭空捏造
3. **必须进行产业链上下游推演**，不能只推荐新闻直接提到的行业，还要覆盖间接受益的配套/上游板块
4. 如果宏观新闻中提到具体涨幅或事件，要在推荐逻辑中体现
5. 【推荐代码】行必须包含所有推荐和提及的股票代码，不要遗漏
6. AI/算力相关新闻必须同时推荐：AI应用层 + 算力硬件 + **电力运营** + **特高压/输变电** + 数据中心/IDC
7. 每个板块推荐要标注产业链位置（如"上游原材料"、"中游制造"、"下游应用"、"配套基建"）
8. **新兴产业链补充**：若新闻涉及模板未覆盖的新兴领域（如低空经济/eVTOL、人形机器人、商业航天/卫星互联网、脑机接口、固态电池、可控核聚变、量子计算、创新药CXO、合成生物），请按同样的上下游推演框架补充
9. **过滤规则（防止追高推荐）**：推荐个股时尽量排除以下情况：
   (a) 近 5 个交易日累计涨幅 > 25% 的品种（已兑现预期，回撤风险大）
   (b) PE > 80 倍且非高速成长（营收/净利润同比 < 30%）的品种
   (c) 已连续 3 个涨停或 5 天内涨停 2 次以上的纯情绪股
   若新闻里的核心龙头已满足以上条件，需在推荐逻辑中主动提示"已兑现预期，关注补涨品种"并给出替代标的
10. **轮动思维**：避免所有推荐集中在同一板块，3-5 个板块推荐应覆盖成长+价值+防御至少 2 类风格
"#,
            today = today,
            macro_news = macro_news
        );

        // 使用宏观推荐专用系统提示词（而非个股决策仪表盘提示词）
        const MACRO_SYSTEM_PROMPT: &str = "\
你是一位资深 A 股宏观策略分析师，专精于自上而下 (Top-Down) 宏观驱动选股，深度理解A股票市场，从业20年，经历过多轮牛熊市，同时持续盈利。\
你的核心能力有四个：\
1. 产业链推演：从一条新闻出发，沿上下游推导出所有受益环节（如 AI/算力爆发→电力需求→特高压→IDC）；\
2. 实际业务需求验证：推荐必须有国内外真实业务需求支撑（全球AI资本开支增长、东数西算工程、新型电力系统建设、欧美碳中和出口需求、国产替代订单等），\
不能只靠概念炒作，要关注产能利用率、订单增速、出口数据、政策落地进度等实际指标。\
3. 市场情绪分析：评估当前市场情绪，判断投资者的乐观或悲观程度，以及潜在的恐慌性抛售或过度追高行为。\
4. 结合过往市场表现，分析当前宏观新闻对不同板块的潜在影响，推荐最有可能受益的板块和个股。\
请基于最新宏观新闻，做深度产业链推演+实际需求验证后，推荐受益的 A 股板块和个股。\
直接以 Markdown 格式输出分析，不要输出 JSON 决策仪表盘。\
回答要简洁高效，重点突出。";

        info!("🤖 正在基于宏观新闻进行 A 股智能推荐...");
        let response = self.call_api_with_retry_ex(&prompt, MACRO_SYSTEM_PROMPT).await?;
        Ok(response)
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

fn get_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn format_volume(volume: Option<f64>) -> String {
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
    ANALYZER.get_or_init(|| {
        std::sync::Mutex::new(GeminiAnalyzer::from_env())
    })
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
