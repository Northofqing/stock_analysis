//! 提示词格式化与响应解析（从 analyzer.rs 拆分）。

use log::warn;
use serde_json::Value;
use std::collections::HashMap;

use super::types::AnalysisResult;
use super::{format_volume, get_string, GeminiAnalyzer};

impl GeminiAnalyzer {
    /// 格式化提示词
    pub(super) fn format_prompt(
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
    pub(super) fn parse_response(&self, response_text: &str, code: &str, name: &str) -> AnalysisResult {
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

}
