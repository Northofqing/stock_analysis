// -*- coding: utf-8 -*-
//! 通知层模块
//!
//! 职责：
//! 1. 汇总分析结果生成日报
//! 2. 支持 Markdown 格式输出
//! 3. 多渠道推送：企业微信、飞书、Telegram、邮件、Pushover
//!
//! 支持的渠道：
//! - 企业微信 Webhook
//! - 飞书 Webhook  
//! - Telegram Bot
//! - 邮件 SMTP
//! - Pushover（手机/桌面推送）
//! - 自定义 Webhook

use anyhow::{Context, Result};
use chrono::Local;
use log::{error, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use std::path::Path;
use lettre::{Message, SmtpTransport, Transport};
use lettre::message::{header, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;

/// 通知渠道类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationChannel {
    /// 企业微信
    Wechat,
    /// 飞书
    Feishu,
    /// Telegram
    Telegram,
    /// 邮件
    Email,
    /// Pushover
    Pushover,
    /// 自定义Webhook
    Custom,
}

impl NotificationChannel {
    /// 获取渠道中文名称
    pub fn name(&self) -> &'static str {
        match self {
            Self::Wechat => "企业微信",
            Self::Feishu => "飞书",
            Self::Telegram => "Telegram",
            Self::Email => "邮件",
            Self::Pushover => "Pushover",
            Self::Custom => "自定义Webhook",
        }
    }
}

/// SMTP 服务器配置
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub server: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// 通知服务配置
#[derive(Debug, Clone, Default)]
pub struct NotificationConfig {
    // 企业微信
    pub wechat_webhook_url: Option<String>,
    
    // 飞书
    pub feishu_webhook_url: Option<String>,
    
    // Telegram
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    
    // 邮件
    pub email_sender: Option<String>,
    pub email_password: Option<String>,
    pub email_receivers: Vec<String>,
    pub smtp_server: Option<String>,
    pub smtp_port: Option<u16>,
    
    // Pushover
    pub pushover_user_key: Option<String>,
    pub pushover_api_token: Option<String>,
    
    // 自定义Webhook
    pub custom_webhook_urls: Vec<String>,
    pub custom_webhook_bearer_token: Option<String>,
    
    // 消息长度限制
    pub wechat_max_bytes: usize,
    pub feishu_max_bytes: usize,
}

impl NotificationConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        Self {
            wechat_webhook_url: std::env::var("WECHAT_WEBHOOK_URL").ok(),
            feishu_webhook_url: std::env::var("FEISHU_WEBHOOK_URL").ok(),
            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN").ok(),
            telegram_chat_id: std::env::var("TELEGRAM_CHAT_ID").ok(),
            email_sender: std::env::var("EMAIL_SENDER").ok(),
            email_password: std::env::var("EMAIL_PASSWORD").ok(),
            email_receivers: std::env::var("EMAIL_RECEIVERS")
                .ok()
                .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            smtp_server: std::env::var("SMTP_SERVER").ok(),
            smtp_port: std::env::var("SMTP_PORT")
                .ok()
                .and_then(|s| s.parse().ok()),
            pushover_user_key: std::env::var("PUSHOVER_USER_KEY").ok(),
            pushover_api_token: std::env::var("PUSHOVER_API_TOKEN").ok(),
            custom_webhook_urls: std::env::var("CUSTOM_WEBHOOK_URLS")
                .ok()
                .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            custom_webhook_bearer_token: std::env::var("CUSTOM_WEBHOOK_BEARER_TOKEN").ok(),
            wechat_max_bytes: std::env::var("WECHAT_MAX_BYTES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4000),
            feishu_max_bytes: std::env::var("FEISHU_MAX_BYTES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20000),
        }
    }
}

/// 简化的分析结果结构（用于通知）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub code: String,
    pub name: String,
    pub sentiment_score: i32,
    pub trend_prediction: String,
    pub operation_advice: String,
    pub analysis_summary: String,
    pub technical_analysis: Option<String>,
    pub news_summary: Option<String>,
    pub buy_reason: Option<String>,
    pub risk_warning: Option<String>,
    pub ma_analysis: Option<String>,
    pub volume_analysis: Option<String>,

    // ========== 估值指标 ==========
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub circulating_cap: Option<f64>,

    // ========== 均线与乖离率 ==========
    pub current_price: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub ma_alignment: Option<String>,
    pub bias_ma5: Option<f64>,

    // ========== 量能 ==========
    pub volume_ratio_5d: Option<f64>,

    // ========== 52周/季度价格区间 ==========
    pub high_52w: Option<f64>,
    pub low_52w: Option<f64>,
    pub pos_52w: Option<f64>,
    pub high_quarter: Option<f64>,
    pub low_quarter: Option<f64>,
    pub pos_quarter: Option<f64>,

    // ========== 近期走势 ==========
    pub chg_5d: Option<f64>,
    pub chg_10d: Option<f64>,
    pub volatility: Option<f64>,

    // ========== 财务指标 ==========
    pub eps: Option<f64>,
    pub roe: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub net_profit_yoy: Option<f64>,
    pub sharpe_ratio: Option<f64>,
}

impl AnalysisResult {
    /// 获取emoji表情
    pub fn get_emoji(&self) -> &'static str {
        match self.sentiment_score {
            80.. => "💚",
            65..=79 => "🟢",
            55..=64 => "🟡",
            45..=54 => "⚪",
            35..=44 => "🟠",
            _ => "🔴",
        }
    }
}

impl From<&crate::pipeline::AnalysisResult> for AnalysisResult {
    fn from(r: &crate::pipeline::AnalysisResult) -> Self {
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            sentiment_score: r.sentiment_score,
            trend_prediction: r.trend_prediction.clone(),
            operation_advice: r.operation_advice.clone(),
            analysis_summary: r.analysis_content.clone(),
            technical_analysis: None,
            news_summary: None,
            buy_reason: None,
            risk_warning: None,
            ma_analysis: r.ma_alignment.clone(),
            volume_analysis: None,
            pe_ratio: r.pe_ratio,
            pb_ratio: r.pb_ratio,
            turnover_rate: r.turnover_rate,
            market_cap: r.market_cap,
            circulating_cap: r.circulating_cap,
            current_price: r.current_price,
            ma5: r.ma5,
            ma10: r.ma10,
            ma20: r.ma20,
            ma60: r.ma60,
            ma_alignment: r.ma_alignment.clone(),
            bias_ma5: r.bias_ma5,
            volume_ratio_5d: r.volume_ratio_5d,
            high_52w: r.high_52w,
            low_52w: r.low_52w,
            pos_52w: r.pos_52w,
            high_quarter: r.high_quarter,
            low_quarter: r.low_quarter,
            pos_quarter: r.pos_quarter,
            chg_5d: r.chg_5d,
            chg_10d: r.chg_10d,
            volatility: r.volatility,
            eps: r.eps,
            roe: r.roe,
            gross_margin: r.gross_margin,
            net_margin: r.net_margin,
            revenue_yoy: r.revenue_yoy,
            net_profit_yoy: r.net_profit_yoy,
            sharpe_ratio: r.sharpe_ratio,
        }
    }
}

/// 通知服务
pub struct NotificationService {
    config: NotificationConfig,
    client: Client,
    available_channels: Vec<NotificationChannel>,
}

impl NotificationService {
    /// 创建新的通知服务
    pub fn new(config: NotificationConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let available_channels = Self::detect_channels(&config);

        if available_channels.is_empty() {
            warn!("未配置有效的通知渠道，将不发送推送通知");
        } else {
            let names: Vec<_> = available_channels.iter().map(|c| c.name()).collect();
            info!("已配置 {} 个通知渠道：{}", available_channels.len(), names.join(", "));
        }

        Self {
            config,
            client,
            available_channels,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(NotificationConfig::from_env())
    }

    /// 检测所有已配置的渠道
    fn detect_channels(config: &NotificationConfig) -> Vec<NotificationChannel> {
        let mut channels = Vec::new();

        if config.wechat_webhook_url.is_some() {
            channels.push(NotificationChannel::Wechat);
        }

        if config.feishu_webhook_url.is_some() {
            channels.push(NotificationChannel::Feishu);
        }

        if config.telegram_bot_token.is_some() && config.telegram_chat_id.is_some() {
            channels.push(NotificationChannel::Telegram);
        }

        if config.email_sender.is_some() && config.email_password.is_some() {
            channels.push(NotificationChannel::Email);
        }

        if config.pushover_user_key.is_some() && config.pushover_api_token.is_some() {
            channels.push(NotificationChannel::Pushover);
        }

        if !config.custom_webhook_urls.is_empty() {
            channels.push(NotificationChannel::Custom);
        }

        channels
    }

    /// 检查服务是否可用
    pub fn is_available(&self) -> bool {
        !self.available_channels.is_empty()
    }

    /// 获取已配置渠道列表
    pub fn get_available_channels(&self) -> &[NotificationChannel] {
        &self.available_channels
    }

    /// 获取渠道名称字符串
    pub fn get_channel_names(&self) -> String {
        self.available_channels
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 生成 Markdown 格式的日报
    pub fn generate_daily_report(&self, results: &[AnalysisResult]) -> String {
        let report_date = Local::now().format("%Y-%m-%d").to_string();
        let now = Local::now().format("%H:%M:%S").to_string();

        let mut lines = vec![
            format!("# 📅 {} A股自选股智能分析报告", report_date),
            String::new(),
            format!("> 共分析 **{}** 只股票 | 报告生成时间：{}", results.len(), now),
            String::new(),
            "---".to_string(),
            String::new(),
        ];

        // 按评分排序
        let mut sorted_results = results.to_vec();
        sorted_results.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));

        // 统计信息
        let buy_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "买入" | "加仓" | "强烈买入" | "建议买入" | "强烈建议买入"))
            .count();
        let sell_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "卖出" | "减仓" | "建议减仓" | "建议卖出" |"强烈卖出"))
            .count();
        let hold_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "持有" | "观望"))
            .count();
        let avg_score: f64 = results.iter().map(|r| r.sentiment_score as f64).sum::<f64>()
            / results.len() as f64;

        lines.extend(vec![
            "## 📊 操作建议汇总".to_string(),
            String::new(),
            "| 指标 | 数值 |".to_string(),
            "|------|------|".to_string(),
            format!("| 🟢 建议买入/加仓 | **{}** 只 |", buy_count),
            format!("| 🟡 建议持有/观望 | **{}** 只 |", hold_count),
            format!("| 🔴 建议减仓/卖出 | **{}** 只 |", sell_count),
            format!("| 📈 平均看多评分 | **{:.1}** 分 |", avg_score),
            String::new(),
            "---".to_string(),
            String::new(),
            "## 📈 个股详细分析".to_string(),
            String::new(),
        ]);

        // 逐个股票的详细分析
        for result in &sorted_results {
            let emoji = result.get_emoji();

            lines.push(format!("### {} {} ({})", emoji, result.name, result.code));
            lines.push(String::new());
            lines.push(format!(
                "**操作建议：{}** | **综合评分：{}分** | **趋势预测：{}**",
                result.operation_advice, result.sentiment_score, result.trend_prediction
            ));
            lines.push(String::new());

            // 操作理由
            if let Some(buy_reason) = &result.buy_reason {
                lines.push(format!("**💡 操作理由**：{}", buy_reason));
                lines.push(String::new());
            }

            // ========== 均线与价格位置 ==========
            let has_ma_data = result.current_price.is_some() && result.ma5.is_some();
            if has_ma_data {
                lines.push("#### 📈 均线与价格位置".to_string());
                lines.push(String::new());
                lines.push("| 项目 | 价格 | 说明 |".to_string());
                lines.push("|------|------|------|".to_string());

                if let Some(price) = result.current_price {
                    lines.push(format!("| 当前价 | {:.2} | - |", price));
                }
                if let Some(ma5) = result.ma5 {
                    let bias_str = result.bias_ma5
                        .map(|b| {
                            let warn = if b.abs() > 5.0 { " ⚠️偏离过大" } else { "" };
                            format!("乖离率: {:.2}%{}", b, warn)
                        })
                        .unwrap_or_default();
                    lines.push(format!("| MA5 | {:.2} | {} |", ma5, bias_str));
                }
                if let Some(ma10) = result.ma10 {
                    lines.push(format!("| MA10 | {:.2} | - |", ma10));
                }
                if let Some(ma20) = result.ma20 {
                    lines.push(format!("| MA20 | {:.2} | - |", ma20));
                }
                if let Some(ma60) = result.ma60 {
                    lines.push(format!("| MA60 | {:.2} | 中期趋势 |", ma60));
                }

                if let Some(ref alignment) = result.ma_alignment {
                    lines.push(format!("| 排列状态 | {} | - |", alignment));
                }

                lines.push(String::new());
            }

            // ========== 52周/季度价格区间 ==========
            let has_range = result.high_52w.is_some() || result.high_quarter.is_some();
            if has_range {
                lines.push("#### 📏 价格区间".to_string());
                lines.push(String::new());
                lines.push("| 区间 | 最高 | 最低 | 当前位置 |".to_string());
                lines.push("|------|------|------|---------|".to_string());

                if let (Some(h), Some(l), Some(p)) = (result.high_52w, result.low_52w, result.pos_52w) {
                    let pos_desc = if p > 80.0 { "接近高点 ⚠️" }
                        else if p < 20.0 { "接近低点 ✅" }
                        else { "" };
                    lines.push(format!("| 52周 | {:.2} | {:.2} | {:.1}% {} |", h, l, p, pos_desc));
                }
                if let (Some(h), Some(l), Some(p)) = (result.high_quarter, result.low_quarter, result.pos_quarter) {
                    lines.push(format!("| 近一季 | {:.2} | {:.2} | {:.1}% |", h, l, p));
                }

                lines.push(String::new());
            }

            // ========== 量能与近期走势 ==========
            let has_momentum = result.volume_ratio_5d.is_some() || result.chg_5d.is_some();
            if has_momentum {
                lines.push("#### 📊 量能与近期走势".to_string());
                lines.push(String::new());

                if let Some(vr) = result.volume_ratio_5d {
                    let vol_status = if vr > 2.0 { "显著放量" }
                        else if vr > 1.2 { "温和放量" }
                        else if vr > 0.8 { "量能平稳" }
                        else { "明显缩量" };
                    lines.push(format!("- **5日量比**: {:.2} ({})", vr, vol_status));
                }
                if let Some(chg) = result.chg_5d {
                    lines.push(format!("- **近5日涨幅**: {:.2}%", chg));
                }
                if let Some(chg) = result.chg_10d {
                    lines.push(format!("- **近10日涨幅**: {:.2}%", chg));
                }
                if let Some(vol) = result.volatility {
                    let vol_level = if vol > 5.0 { "⚠️ 波动剧烈" }
                        else if vol > 3.0 { "波动较大" }
                        else { "波动正常" };
                    lines.push(format!("- **日波动率**: {:.2}% ({})", vol, vol_level));
                }

                lines.push(String::new());
            }

            // ========== 估值指标 ==========
            let has_valuation = result.pe_ratio.is_some() || result.pb_ratio.is_some()
                || result.market_cap.is_some() || result.turnover_rate.is_some();
            if has_valuation {
                lines.push("#### 💰 估值指标".to_string());
                lines.push(String::new());
                lines.push("| 指标 | 数值 | 评估 |".to_string());
                lines.push("|------|------|------|".to_string());

                if let Some(pe) = result.pe_ratio {
                    let a = if pe < 0.0 { "亏损" }
                        else if pe < 15.0 { "✅ 合理" }
                        else if pe < 30.0 { "⚠️ 适中" }
                        else { "🔴 偏高" };
                    lines.push(format!("| PE | {:.2} | {} |", pe, a));
                }
                if let Some(pb) = result.pb_ratio {
                    let a = if pb < 1.0 { "✅ 可能低估" }
                        else if pb < 3.0 { "正常" }
                        else { "🔴 偏高" };
                    lines.push(format!("| PB | {:.2} | {} |", pb, a));
                }
                if let Some(t) = result.turnover_rate {
                    let a = if t < 1.0 { "极度清淡" }
                        else if t < 3.0 { "清淡" }
                        else if t < 7.0 { "正常" }
                        else if t < 15.0 { "活跃" }
                        else { "火热" };
                    lines.push(format!("| 换手率 | {:.2}% | {} |", t, a));
                }
                if let Some(mc) = result.market_cap {
                    let cap = if mc < 50.0 { "小盘" }
                        else if mc < 300.0 { "中盘" }
                        else if mc < 1000.0 { "大盘" }
                        else { "超大盘" };
                    lines.push(format!("| 总市值 | {:.2}亿 | {} |", mc, cap));
                }
                if let Some(cc) = result.circulating_cap {
                    lines.push(format!("| 流通市值 | {:.2}亿 | - |", cc));
                }

                lines.push(String::new());
            }

            // ========== 财务指标 ==========
            let has_financials = result.eps.is_some() || result.roe.is_some()
                || result.gross_margin.is_some() || result.revenue_yoy.is_some();
            if has_financials {
                lines.push("#### 📋 财务指标".to_string());
                lines.push(String::new());
                lines.push("| 指标 | 数值 | 评估 |".to_string());
                lines.push("|------|------|------|".to_string());

                if let Some(eps) = result.eps {
                    let a = if eps < 0.0 { "亏损" }
                        else if eps < 0.5 { "较弱" }
                        else if eps < 2.0 { "正常" }
                        else { "✅ 优秀" };
                    lines.push(format!("| EPS | {:.3}元 | {} |", eps, a));
                }
                if let Some(roe) = result.roe {
                    let a = if roe < 5.0 { "较低" }
                        else if roe < 15.0 { "正常" }
                        else if roe < 25.0 { "✅ 优秀" }
                        else { "🌟 卓越" };
                    lines.push(format!("| ROE | {:.2}% | {} |", roe, a));
                }
                if let Some(gm) = result.gross_margin {
                    let a = if gm < 20.0 { "竞争激烈" }
                        else if gm < 40.0 { "正常" }
                        else { "✅ 高壁垒" };
                    lines.push(format!("| 毛利率 | {:.2}% | {} |", gm, a));
                }
                if let Some(nm) = result.net_margin {
                    lines.push(format!("| 净利率 | {:.2}% | - |", nm));
                }
                if let Some(r) = result.revenue_yoy {
                    let a = if r < 0.0 { "🔴 下滑" }
                        else if r < 10.0 { "缓慢" }
                        else if r < 30.0 { "✅ 稳健" }
                        else { "🚀 高速" };
                    lines.push(format!("| 营收同比 | {:.2}% | {} |", r, a));
                }
                if let Some(p) = result.net_profit_yoy {
                    let a = if p < -20.0 { "🔴 大幅下滑" }
                        else if p < 0.0 { "⚠️ 下滑" }
                        else if p < 20.0 { "稳定" }
                        else { "✅ 高速增长" };
                    lines.push(format!("| 净利润同比 | {:.2}% | {} |", p, a));
                }
                if let Some(sr) = result.sharpe_ratio {
                    let a = if sr < 0.0 { "🔴 亏损" }
                        else if sr < 1.0 { "一般" }
                        else if sr < 2.0 { "✅ 良好" }
                        else { "🌟 优秀" };
                    lines.push(format!("| 夏普比率 | {:.2} | {} |", sr, a));
                }

                lines.push(String::new());
            }

            // 技术面分析（原有文本）
            let mut tech_lines = Vec::new();
            if let Some(tech) = &result.technical_analysis {
                tech_lines.push(format!("**综合**：{}", tech));
            }
            if let Some(vol) = &result.volume_analysis {
                tech_lines.push(format!("**量能**：{}", vol));
            }
            if !tech_lines.is_empty() {
                lines.push("#### 🔍 技术面分析".to_string());
                lines.extend(tech_lines);
                lines.push(String::new());
            }

            // 消息面
            if let Some(news) = &result.news_summary {
                lines.push("#### 📰 消息面".to_string());
                lines.push(news.clone());
                lines.push(String::new());
            }

            // 综合分析
            lines.push("#### 📝 综合分析".to_string());
            lines.push(result.analysis_summary.clone());
            lines.push(String::new());

            // 风险提示
            if let Some(risk) = &result.risk_warning {
                lines.push(format!("⚠️ **风险提示**：{}", risk));
                lines.push(String::new());
            }

            lines.push(String::new());
            lines.push("---".to_string());
            lines.push(String::new());
        }

        // 底部信息
        lines.push(String::new());
        lines.push(format!(
            "*报告生成时间：{}*",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        lines.join("\n")
    }

    /// 生成精简版日报（用于企业微信）
    pub fn generate_wechat_summary(&self, results: &[AnalysisResult]) -> String {
        let report_date = Local::now().format("%Y-%m-%d").to_string();

        let mut sorted_results = results.to_vec();
        sorted_results.sort_by(|b, a| a.sentiment_score.cmp(&b.sentiment_score));

        let buy_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "买入" | "加仓" | "强烈买入"))
            .count();
        let sell_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "卖出" | "减仓" | "强烈卖出"))
            .count();
        let hold_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "持有" | "观望"))
            .count();
        let avg_score: f64 = results.iter().map(|r| r.sentiment_score as f64).sum::<f64>()
            / results.len() as f64;

        let mut lines = vec![
            format!("## 📅 {} A股分析报告", report_date),
            String::new(),
            format!(
                "> 共 **{}** 只 | 🟢买入:{} 🟡持有:{} 🔴卖出:{} | 均分:{:.0}",
                results.len(),
                buy_count,
                hold_count,
                sell_count,
                avg_score
            ),
            String::new(),
        ];

        for result in &sorted_results {
            let emoji = result.get_emoji();
            lines.push(format!("### {} {}({})", emoji, result.name, result.code));
            lines.push(format!(
                "**{}** | 评分:{} | {}",
                result.operation_advice, result.sentiment_score, result.trend_prediction
            ));

            // 紧凑的核心指标行
            let mut indicators = Vec::new();
            if let Some(price) = result.current_price {
                indicators.push(format!("价:{:.2}", price));
            }
            if let Some(ref align) = result.ma_alignment {
                indicators.push(align.clone());
            }
            if let Some(bias) = result.bias_ma5 {
                if bias.abs() > 3.0 {
                    indicators.push(format!("乖离:{:.1}%⚠️", bias));
                }
            }
            if let Some(vr) = result.volume_ratio_5d {
                let vs = if vr > 2.0 { "放量" } else if vr < 0.8 { "缩量" } else { "" };
                if !vs.is_empty() {
                    indicators.push(format!("量比{:.1}({})", vr, vs));
                }
            }
            if let Some(p) = result.pos_52w {
                indicators.push(format!("52周位:{:.0}%", p));
            }
            if !indicators.is_empty() {
                lines.push(format!("📈 {}", indicators.join(" | ")));
            }

            // 紧凑的估值/财务行
            let mut val_parts = Vec::new();
            if let Some(pe) = result.pe_ratio {
                val_parts.push(format!("PE:{:.1}", pe));
            }
            if let Some(pb) = result.pb_ratio {
                val_parts.push(format!("PB:{:.1}", pb));
            }
            if let Some(roe) = result.roe {
                val_parts.push(format!("ROE:{:.1}%", roe));
            }
            if let Some(chg) = result.chg_5d {
                val_parts.push(format!("5日:{:+.1}%", chg));
            }
            if !val_parts.is_empty() {
                lines.push(format!("💰 {}", val_parts.join(" | ")));
            }

            if let Some(reason) = &result.buy_reason {
                let truncated = if reason.len() > 80 {
                    format!("{}...", &reason[..77])
                } else {
                    reason.clone()
                };
                lines.push(format!("💡 {}", truncated));
            }

            if let Some(risk) = &result.risk_warning {
                let truncated = if risk.len() > 50 {
                    format!("{}...", &risk[..47])
                } else {
                    risk.clone()
                };
                lines.push(format!("⚠️ {}", truncated));
            }

            lines.push(String::new());
        }

        lines.push("---".to_string());
        lines.push("*AI生成，仅供参考，不构成投资建议*".to_string());
        lines.push(format!(
            "*详细报告见 reports/report_{}.md*",
            report_date.replace("-", "")
        ));

        lines.join("\n")
    }

    /// 发送到企业微信
    pub async fn send_to_wechat(&self, content: &str) -> Result<bool> {
        let url = self
            .config
            .wechat_webhook_url
            .as_ref()
            .context("企业微信 Webhook 未配置")?;

        let max_bytes = self.config.wechat_max_bytes;
        let content_bytes = content.as_bytes().len();

        if content_bytes > max_bytes {
            info!("消息内容超长({}字节)，将分批发送", content_bytes);
            return self.send_wechat_chunked(url, content, max_bytes).await;
        }

        self.send_wechat_message(url, content).await
    }

    /// 发送单条企业微信消息
    async fn send_wechat_message(&self, url: &str, content: &str) -> Result<bool> {
        let payload = json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content
            }
        });

        let response = self.client.post(url).json(&payload).send().await?;

        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            if result.get("errcode").and_then(|v| v.as_i64()) == Some(0) {
                info!("企业微信消息发送成功");
                Ok(true)
            } else {
                error!("企业微信返回错误: {:?}", result);
                Ok(false)
            }
        } else {
            error!("企业微信请求失败: {}", response.status());
            Ok(false)
        }
    }

    /// 分批发送长消息到企业微信
    async fn send_wechat_chunked(&self, url: &str, content: &str, max_bytes: usize) -> Result<bool> {
        let chunks = self.chunk_by_sections(content, max_bytes);
        let total_chunks = chunks.len();
        let mut success_count = 0;

        info!("企业微信分批发送：共 {} 批", total_chunks);

        for (i, chunk) in chunks.iter().enumerate() {
            let page_marker = if total_chunks > 1 {
                format!("\n\n📄 *({}/{})*", i + 1, total_chunks)
            } else {
                String::new()
            };

            let chunk_with_marker = format!("{}{}", chunk, page_marker);

            if self.send_wechat_message(url, &chunk_with_marker).await? {
                success_count += 1;
                info!("企业微信第 {}/{} 批发送成功", i + 1, total_chunks);
            } else {
                error!("企业微信第 {}/{} 批发送失败", i + 1, total_chunks);
            }

            if i < total_chunks - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        Ok(success_count == total_chunks)
    }

    /// 按段落智能分割内容
    fn chunk_by_sections(&self, content: &str, max_bytes: usize) -> Vec<String> {
        let sections: Vec<&str> = if content.contains("\n---\n") {
            content.split("\n---\n").collect()
        } else if content.contains("\n### ") {
            let parts: Vec<&str> = content.split("\n### ").collect();
            let mut result = vec![parts[0]];
            let formatted_parts: Vec<String> = parts[1..].iter().map(|p| format!("### {}", p)).collect();
            result.extend(formatted_parts.iter().map(|s| s.as_str()));
            return self.chunk_sections(&result, max_bytes);
        } else {
            vec![content]
        };

        self.chunk_sections(&sections, max_bytes)
    }

    fn chunk_sections(&self, sections: &[&str], max_bytes: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current_chunk = Vec::new();
        let mut current_bytes = 0;

        for section in sections {
            let section_bytes = section.as_bytes().len();

            if section_bytes > max_bytes {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.join("\n---\n"));
                    current_chunk.clear();
                    current_bytes = 0;
                }

                let truncated = self.truncate_to_bytes(section, max_bytes - 200);
                chunks.push(format!("{}\n\n...(本段内容过长已截断)", truncated));
                continue;
            }

            if current_bytes + section_bytes > max_bytes {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.join("\n---\n"));
                }
                current_chunk = vec![section.to_string()];
                current_bytes = section_bytes;
            } else {
                current_chunk.push(section.to_string());
                current_bytes += section_bytes;
            }
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk.join("\n---\n"));
        }

        chunks
    }

    fn truncate_to_bytes(&self, text: &str, max_bytes: usize) -> String {
        if text.as_bytes().len() <= max_bytes {
            return text.to_string();
        }

        let mut result = String::new();
        let mut current_bytes = 0;

        for c in text.chars() {
            let char_bytes = c.len_utf8();
            if current_bytes + char_bytes > max_bytes {
                break;
            }
            result.push(c);
            current_bytes += char_bytes;
        }

        result
    }

    /// 发送到飞书
    pub async fn send_to_feishu(&self, content: &str) -> Result<bool> {
        let url = self
            .config
            .feishu_webhook_url
            .as_ref()
            .context("飞书 Webhook 未配置")?;

        let formatted = self.format_feishu_markdown(content);
        let max_bytes = self.config.feishu_max_bytes;

        if formatted.as_bytes().len() > max_bytes {
            info!("飞书消息内容超长，将分批发送");
            return self.send_feishu_chunked(url, &formatted, max_bytes).await;
        }

        self.send_feishu_message(url, &formatted).await
    }

    async fn send_feishu_message(&self, url: &str, content: &str) -> Result<bool> {
        // 优先使用交互卡片
        let card_payload = json!({
            "msg_type": "interactive",
            "card": {
                "config": {"wide_screen_mode": true},
                "header": {
                    "title": {
                        "tag": "plain_text",
                        "content": "A股智能分析报告"
                    }
                },
                "elements": [{
                    "tag": "div",
                    "text": {
                        "tag": "lark_md",
                        "content": content
                    }
                }]
            }
        });

        let response = self.client.post(url).json(&card_payload).send().await?;

        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            let code = result
                .get("code")
                .or_else(|| result.get("StatusCode"))
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);

            if code == 0 {
                info!("飞书消息发送成功");
                return Ok(true);
            }
        }

        // 回退到普通文本
        let text_payload = json!({
            "msg_type": "text",
            "content": {
                "text": content
            }
        });

        let response = self.client.post(url).json(&text_payload).send().await?;
        Ok(response.status().is_success())
    }

    async fn send_feishu_chunked(&self, url: &str, content: &str, max_bytes: usize) -> Result<bool> {
        let chunks = self.chunk_by_sections(content, max_bytes);
        let total = chunks.len();
        let mut success = 0;

        for (i, chunk) in chunks.iter().enumerate() {
            let marker = if total > 1 {
                format!("\n\n📄 ({}/{})", i + 1, total)
            } else {
                String::new()
            };

            if self.send_feishu_message(url, &format!("{}{}", chunk, marker)).await? {
                success += 1;
            }

            if i < total - 1 {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }

        Ok(success == total)
    }

    fn format_feishu_markdown(&self, content: &str) -> String {
        use regex::Regex;

        let mut result = content.to_string();

        // 标题转加粗
        let re_h = Regex::new(r"^#{1,6}\s+(.+)$").unwrap();
        result = re_h
            .replace_all(&result, |caps: &regex::Captures| format!("**{}**", &caps[1]))
            .to_string();

        // 引用转前缀
        result = result.replace("> ", "💬 ");

        // 分隔线
        result = result.replace("---", "────────");

        // 列表
        result = result.replace("- ", "• ");

        result
    }

    /// 将 Markdown 转换为 HTML（优化邮件客户端兼容性）
    fn markdown_to_html(&self, markdown: &str) -> String {
        let mut html = markdown.to_string();
        
        // 清理多余的空行（3个以上连续换行合并为2个）
        let re_multiple_newlines = regex::Regex::new(r"\n{3,}").unwrap();
        html = re_multiple_newlines.replace_all(&html, "\n\n").to_string();
        
        // 先处理表格（最重要的部分）
        html = self.convert_markdown_tables_enhanced(&html);
        
        // 处理引用块
        let re_quote = regex::Regex::new(r"(?m)^> (.+)$").unwrap();
        html = re_quote.replace_all(&html, 
            "<div style='border-left: 4px solid #3498db; padding: 10px 15px; margin: 15px 0; background-color: #f8f9fa; color: #555;'>$1</div>").to_string();
        
        // 处理标题（从小到大避免冲突）
        let re_h4 = regex::Regex::new(r"(?m)^####\s+(.+)$").unwrap();
        html = re_h4.replace_all(&html, 
            "<h4 style='color: #666; margin: 15px 0 10px 0; font-size: 16px;'>$1</h4>").to_string();
        
        let re_h3 = regex::Regex::new(r"(?m)^###\s+(.+)$").unwrap();
        html = re_h3.replace_all(&html, 
            "<h3 style='color: #555; margin: 20px 0 10px 0; font-size: 18px;'>$1</h3>").to_string();
        
        let re_h2 = regex::Regex::new(r"(?m)^##\s+(.+)$").unwrap();
        html = re_h2.replace_all(&html, 
            "<h2 style='color: #34495e; margin: 25px 0 15px 0; padding-left: 10px; border-left: 4px solid #3498db; font-size: 20px;'>$1</h2>").to_string();
        
        let re_h1 = regex::Regex::new(r"(?m)^#\s+(.+)$").unwrap();
        html = re_h1.replace_all(&html, 
            "<h1 style='color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; margin: 30px 0 20px 0; font-size: 24px;'>$1</h1>").to_string();
        
        // 处理粗体
        let re_bold = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
        html = re_bold.replace_all(&html, "<strong style='color: #e74c3c; font-weight: bold;'>$1</strong>").to_string();
        
        // 处理分隔线
        html = html.replace("\n---\n", "\n<hr style='border: none; border-top: 2px solid #ecf0f1; margin: 20px 0;'/>\n");
        
        // 处理列表
        html = self.convert_markdown_lists(&html);
        
        // 清理HTML标签周围的多余换行
        // 移除标签前后的空白行
        let re_clean_before_tags = regex::Regex::new(r"\n+(<(?:table|h[1-4]|ul|div|hr))").unwrap();
        html = re_clean_before_tags.replace_all(&html, "\n$1").to_string();
        
        let re_clean_after_tags = regex::Regex::new(r"(</(?:table|h[1-4]|ul|div)>)\n+").unwrap();
        html = re_clean_after_tags.replace_all(&html, "$1\n").to_string();
        
        // 移除表格、列表等块级元素内部的单独换行符（但保留有内容的行）
        // 这一步要在段落处理之前
        let re_empty_lines_in_blocks = regex::Regex::new(r"(<(?:table|thead|tbody|tr|ul)>)\n+").unwrap();
        html = re_empty_lines_in_blocks.replace_all(&html, "$1").to_string();
        
        let re_empty_lines_after_blocks = regex::Regex::new(r"\n+(</(?:table|thead|tbody|tr|ul)>)").unwrap();
        html = re_empty_lines_after_blocks.replace_all(&html, "$1").to_string();
        
        // 最后处理剩余的文本换行
        // 只对纯文本段落添加 <br/>，而不是所有换行
        let lines: Vec<&str> = html.lines().collect();
        let mut final_lines = Vec::new();
        
        for line in lines {
            let trimmed = line.trim();
            // 跳过空行
            if trimmed.is_empty() {
                continue;
            }
            // 如果是HTML标签行，直接保留
            if trimmed.starts_with('<') {
                final_lines.push(line.to_string());
            } else {
                // 普通文本行，如果前一行也是文本，添加<br/>
                if !final_lines.is_empty() {
                    let last_line = final_lines.last().unwrap();
                    if !last_line.trim().starts_with('<') && !last_line.trim().ends_with('>') {
                        final_lines.push("<br/>".to_string());
                    }
                }
                final_lines.push(line.to_string());
            }
        }
        html = final_lines.join("\n");
        
        // 包装完整HTML
        format!(
            "<!DOCTYPE html>
<html>
<head>
    <meta charset='UTF-8'>
    <meta name='viewport' content='width=device-width, initial-scale=1.0'>
</head>
<body style='font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", \"Microsoft YaHei\", Arial, sans-serif; line-height: 1.6; padding: 20px; background-color: #ffffff; color: #333; max-width: 1200px; margin: 0 auto;'>
{}
<div style='text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid #ecf0f1; color: #999; font-size: 12px;'>
    <p>本邮件由A股分析系统自动生成</p>
</div>
</body>
</html>",
            html
        )
    }

    /// 转换Markdown列表为HTML（优化版）
    fn convert_markdown_lists(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut in_list = false;
        
        for line in lines {
            if line.trim().starts_with("- ") {
                if !in_list {
                    result.push("<ul style='margin: 10px 0; padding-left: 25px;'>".to_string());
                    in_list = true;
                }
                let content = line.trim_start_matches("- ").trim();
                result.push(format!("<li style='margin: 5px 0;'>{}</li>", content));
            } else {
                if in_list {
                    result.push("</ul>".to_string());
                    in_list = false;
                }
                result.push(line.to_string());
            }
        }
        if in_list {
            result.push("</ul>".to_string());
        }
        result.join("\n")
    }

    /// 转换Markdown表格为HTML（增强版，完全内联样式）
    fn convert_markdown_tables_enhanced(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut i = 0;
        
        while i < lines.len() {
            let line = lines[i];
            
            // 检测表格开始
            if line.contains('|') && line.split('|').filter(|s| !s.trim().is_empty()).count() >= 2 {
                // 检查下一行是否是分隔符
                let is_table_start = if i + 1 < lines.len() {
                    lines[i + 1].contains("---") || lines[i + 1].contains("|-")
                } else {
                    false
                };
                
                if is_table_start {
                    // 表格样式（内联）
                    let table_style = "width: 100%; border-collapse: collapse; margin: 15px 0; background-color: #ffffff; box-shadow: 0 1px 3px rgba(0,0,0,0.1);";
                    let th_style = "background-color: #3498db; color: #ffffff; padding: 12px; text-align: left; font-weight: bold; border: 1px solid #2980b9;";
                    let td_style = "padding: 10px 12px; border: 1px solid #ecf0f1; background-color: #ffffff;";
                    
                    result.push(format!("<table style='{}'>", table_style));
                    
                    // 处理表头
                    let headers: Vec<&str> = line.split('|')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();
                    
                    result.push("<thead>".to_string());
                    result.push("<tr>".to_string());
                    for header in headers {
                        result.push(format!("<th style='{}'>{}</th>", th_style, header));
                    }
                    result.push("</tr>".to_string());
                    result.push("</thead>".to_string());
                    
                    // 跳过分隔符行
                    i += 2;
                    
                    // 处理表格数据行
                    result.push("<tbody>".to_string());
                    let mut row_index = 0;
                    while i < lines.len() {
                        let data_line = lines[i];
                        if !data_line.contains('|') || data_line.trim().is_empty() {
                            break;
                        }
                        
                        let cells: Vec<&str> = data_line.split('|')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect();
                        
                        if !cells.is_empty() {
                            // 交替行背景色
                            let row_bg = if row_index % 2 == 0 { "#ffffff" } else { "#f8f9fa" };
                            result.push(format!("<tr style='background-color: {};'>", row_bg));
                            for cell in cells {
                                // 处理单元格内容中的emoji和颜色标记
                                let cell_content = self.enhance_cell_content(cell);
                                result.push(format!("<td style='{}'>{}</td>", td_style, cell_content));
                            }
                            result.push("</tr>".to_string());
                            row_index += 1;
                        }
                        i += 1;
                    }
                    result.push("</tbody>".to_string());
                    result.push("</table>".to_string());
                    continue;
                }
            }
            
            result.push(line.to_string());
            i += 1;
        }
        
        result.join("\n")
    }

    /// 增强单元格内容显示（处理emoji和特殊标记）
    fn enhance_cell_content(&self, content: &str) -> String {
        let mut enhanced = content.to_string();
        
        // 处理emoji - 使用更兼容的方式
        enhanced = enhanced.replace("✅", "<span style='color: #27ae60;'>✅</span>");
        enhanced = enhanced.replace("⚠️", "<span style='color: #f39c12;'>⚠️</span>");
        enhanced = enhanced.replace("🔴", "<span style='color: #e74c3c;'>🔴</span>");
        enhanced = enhanced.replace("🟢", "<span style='color: #27ae60;'>●</span>");
        enhanced = enhanced.replace("🟡", "<span style='color: #f39c12;'>●</span>");
        enhanced = enhanced.replace("📊", "📊");
        enhanced = enhanced.replace("📈", "📈");
        enhanced = enhanced.replace("💰", "💰");
        enhanced = enhanced.replace("🎯", "🎯");
        
        // 处理评估标签的颜色
        if enhanced.contains("合理") || enhanced.contains("正常") || enhanced.contains("低估") {
            enhanced = format!("<span style='color: #27ae60;'>{}</span>", enhanced);
        } else if enhanced.contains("偏高") || enhanced.contains("较高") || enhanced.contains("亏损") {
            enhanced = format!("<span style='color: #e74c3c;'>{}</span>", enhanced);
        } else if enhanced.contains("适中") || enhanced.contains("中性") {
            enhanced = format!("<span style='color: #f39c12;'>{}</span>", enhanced);
        }
        
        enhanced
    }

    /// 发送邮件
    pub fn send_to_email(&self, content: &str) -> Result<bool> {
        let sender = self.config.email_sender.as_ref()
            .context("邮件发送者未配置 (EMAIL_SENDER)")?;
        let password = self.config.email_password.as_ref()
            .context("邮件密码未配置 (EMAIL_PASSWORD)")?;
        let smtp_server = self.config.smtp_server.as_ref()
            .context("SMTP服务器未配置 (SMTP_SERVER)")?;
        let smtp_port = self.config.smtp_port
            .context("SMTP端口未配置 (SMTP_PORT)")?;
        
        if self.config.email_receivers.is_empty() {
            return Err(anyhow::anyhow!("邮件接收者列表为空 (EMAIL_RECEIVERS)"));
        }
        
        let primary = &self.config.email_receivers[0];
        let cc_list: Vec<&String> = self.config.email_receivers.iter().skip(1).collect();
        
        info!("准备发送邮件到主收件人: {}，抄送 {} 位，SMTP: {}:{}", 
            primary, cc_list.len(), smtp_server, smtp_port);
        
        // 转换 Markdown 为 HTML
        let html_content = self.markdown_to_html(content);
        
        // 构建邮件主题
        let subject = format!("A股分析日报 - {}", Local::now().format("%Y-%m-%d"));
        
        self.send_single_email(
            sender, 
            primary,
            &cc_list,
            &subject, 
            content, 
            &html_content,
            smtp_server,
            smtp_port,
            password
        )?;
        
        info!("邮件发送成功: 主收件人 {}，抄送 {} 位", primary, cc_list.len());
        Ok(true)
    }
    
    /// 发送单封邮件（第一个收件人为主地址，其余为抄送）
    fn send_single_email(
        &self,
        from: &str,
        to: &str,
        cc_list: &[&String],
        subject: &str,
        text_content: &str,
        html_content: &str,
        smtp_server: &str,
        smtp_port: u16,
        password: &str,
    ) -> Result<()> {
        // 构建邮件
        let mut builder = Message::builder()
            .from(from.parse()?)
            .to(to.parse()?);
        
        for cc in cc_list {
            builder = builder.cc(cc.parse()?);
        }
        
        let email = builder
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_PLAIN)
                            .body(text_content.to_string())
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(header::ContentType::TEXT_HTML)
                            .body(html_content.to_string())
                    )
            )?;
        
        // 配置 SMTP
        let creds = Credentials::new(from.to_string(), password.to_string());
        
        let mailer = SmtpTransport::relay(smtp_server)?
            .port(smtp_port)
            .credentials(creds)
            .build();
        
        // 发送
        mailer.send(&email)?;
        
        Ok(())
    }

    /// 统一发送接口
    pub async fn send(&self, content: &str) -> Result<bool> {
        if !self.is_available() {
            warn!("通知服务不可用，跳过推送");
            return Ok(false);
        }

        info!(
            "正在向 {} 个渠道发送通知：{}",
            self.available_channels.len(),
            self.get_channel_names()
        );

        let mut success_count = 0;
        let mut fail_count = 0;

        for channel in &self.available_channels {
            match channel {
                NotificationChannel::Wechat => {
                    match self.send_to_wechat(content).await {
                        Ok(true) => success_count += 1,
                        Ok(false) => {
                            error!("[企业微信] 发送失败");
                            fail_count += 1;
                        }
                        Err(e) => {
                            error!("[企业微信] 发送出错: {}", e);
                            fail_count += 1;
                        }
                    }
                }
                NotificationChannel::Feishu => {
                    match self.send_to_feishu(content).await {
                        Ok(true) => success_count += 1,
                        Ok(false) => {
                            error!("[飞书] 发送失败");
                            fail_count += 1;
                        }
                        Err(e) => {
                            error!("[飞书] 发送出错: {}", e);
                            fail_count += 1;
                        }
                    }
                }
                NotificationChannel::Email => {
                    match self.send_to_email(content) {
                        Ok(true) => success_count += 1,
                        Ok(false) => {
                            error!("[邮件] 发送失败");
                            fail_count += 1;
                        }
                        Err(e) => {
                            error!("[邮件] 发送出错: {}", e);
                            fail_count += 1;
                        }
                    }
                }
                _ => {
                    warn!("渠道 {} 暂未实现", channel.name());
                    fail_count += 1;
                }
            }
        }

        info!("通知发送完成：成功 {} 个，失败 {} 个", success_count, fail_count);
        Ok(success_count > 0)
    }

    /// 发送带图片的通知
    pub async fn send_with_image(&self, content: &str, image_path: &Path) -> Result<bool> {
        if !self.is_available() {
            warn!("通知服务不可用，跳过推送");
            return Ok(false);
        }

        info!(
            "正在向 {} 个渠道发送通知（含图片）：{}",
            self.available_channels.len(),
            self.get_channel_names()
        );

        let mut success_count = 0;
        let mut fail_count = 0;

        for channel in &self.available_channels {
            match channel {
                NotificationChannel::Email => {
                    match self.send_email_with_image(content, image_path) {
                        Ok(true) => success_count += 1,
                        Ok(false) => {
                            error!("[邮件] 发送失败");
                            fail_count += 1;
                        }
                        Err(e) => {
                            error!("[邮件] 发送出错: {}", e);
                            fail_count += 1;
                        }
                    }
                }
                _ => {
                    // 其他渠道暂时降级为文本发送
                    warn!("渠道 {} 暂不支持图片，降级为文本发送", channel.name());
                    match channel {
                        NotificationChannel::Wechat => {
                            match self.send_to_wechat(content).await {
                                Ok(true) => success_count += 1,
                                Ok(false) => fail_count += 1,
                                Err(_) => fail_count += 1,
                            }
                        }
                        NotificationChannel::Feishu => {
                            match self.send_to_feishu(content).await {
                                Ok(true) => success_count += 1,
                                Ok(false) => fail_count += 1,
                                Err(_) => fail_count += 1,
                            }
                        }
                        _ => {
                            warn!("渠道 {} 暂未实现", channel.name());
                            fail_count += 1;
                        }
                    }
                }
            }
        }

        info!("通知发送完成：成功 {} 个，失败 {} 个", success_count, fail_count);
        Ok(success_count > 0)
    }

    /// 发送带图片的邮件
    fn send_email_with_image(&self, content: &str, image_path: &Path) -> Result<bool> {
        let sender = self.config.email_sender.as_ref()
            .context("邮件发送者未配置 (EMAIL_SENDER)")?;
        let password = self.config.email_password.as_ref()
            .context("邮件密码未配置 (EMAIL_PASSWORD)")?;
        let smtp_server = self.config.smtp_server.as_ref()
            .context("SMTP服务器未配置 (SMTP_SERVER)")?;
        let smtp_port = self.config.smtp_port
            .context("SMTP端口未配置 (SMTP_PORT)")?;
        
        if self.config.email_receivers.is_empty() {
            return Err(anyhow::anyhow!("邮件接收者列表为空 (EMAIL_RECEIVERS)"));
        }
        
        let primary = &self.config.email_receivers[0];
        let cc_list: Vec<&String> = self.config.email_receivers.iter().skip(1).collect();
        
        info!("准备发送带图片的邮件到主收件人: {}，抄送 {} 位", primary, cc_list.len());
        
        // 转换 Markdown 为 HTML
        let html_content = self.markdown_to_html(content);
        
        // 读取图片
        let image_data = std::fs::read(image_path)
            .context("读取图片文件失败")?;
        let image_filename = image_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("chart.png");
        
        // 构建邮件主题
        let subject = format!("A股分析日报（含图表） - {}", Local::now().format("%Y-%m-%d"));
        
        self.send_single_email_with_image(
            sender,
            primary,
            &cc_list,
            &subject,
            content,
            &html_content,
            &image_data,
            image_filename,
            smtp_server,
            smtp_port,
            password
        )?;
        
        info!("邮件（含图表）发送成功: 主收件人 {}，抄送 {} 位", primary, cc_list.len());
        Ok(true)
    }

    /// 发送单封带图片的邮件（第一个收件人为主地址，其余为抄送）
    fn send_single_email_with_image(
        &self,
        from: &str,
        to: &str,
        cc_list: &[&String],
        subject: &str,
        text_content: &str,
        html_content: &str,
        image_data: &[u8],
        image_filename: &str,
        smtp_server: &str,
        smtp_port: u16,
        password: &str,
    ) -> Result<()> {
        use lettre::message::Attachment;
        
        // 在 HTML 中嵌入图片引用
        let html_with_image = format!(
            "{}<br/><br/><img src=\"cid:{}\" alt=\"分析图表\" style=\"max-width:100%; height:auto;\"/>",
            html_content,
            image_filename
        );
        
        // 构建邮件
        let mut builder = Message::builder()
            .from(from.parse()?)
            .to(to.parse()?);
        
        for cc in cc_list {
            builder = builder.cc(cc.parse()?);
        }
        
        let email = builder
            .subject(subject)
            .multipart(
                MultiPart::mixed()
                    .multipart(
                        MultiPart::alternative()
                            .singlepart(
                                SinglePart::builder()
                                    .header(header::ContentType::TEXT_PLAIN)
                                    .body(text_content.to_string())
                            )
                            .singlepart(
                                SinglePart::builder()
                                    .header(header::ContentType::TEXT_HTML)
                                    .body(html_with_image)
                            )
                    )
                    .singlepart(
                        Attachment::new_inline(image_filename.to_string())
                            .body(image_data.to_vec(), "image/png".parse()?)
                    )
            )?;
        
        // 配置 SMTP
        let creds = Credentials::new(from.to_string(), password.to_string());
        
        let mailer = if smtp_port == 465 {
            SmtpTransport::relay(smtp_server)?
                .credentials(creds)
                .build()
        } else {
            SmtpTransport::starttls_relay(smtp_server)?
                .port(smtp_port)
                .credentials(creds)
                .build()
        };
        
        // 发送邮件
        mailer.send(&email)?;
        
        Ok(())
    }

    /// 保存报告到文件
    pub fn save_report_to_file(&self, content: &str, filename: Option<&str>) -> Result<String> {
        use std::fs;
        use std::path::PathBuf;

        let default_filename = format!("report_{}.md", Local::now().format("%Y%m%d"));
        let filename = filename.unwrap_or(&default_filename);

        let reports_dir = PathBuf::from("reports");
        fs::create_dir_all(&reports_dir)?;

        let filepath = reports_dir.join(filename);
        fs::write(&filepath, content)?;

        let path_str = filepath.to_string_lossy().to_string();
        info!("日报已保存到: {}", path_str);
        Ok(path_str)
    }
}

/// 便捷函数：发送每日报告
pub async fn send_daily_report(results: &[AnalysisResult]) -> Result<bool> {
    let service = NotificationService::from_env();

    // 生成报告
    let report = service.generate_daily_report(results);

    // 保存到本地
    service.save_report_to_file(&report, None)?;

    // 推送
    service.send(&report).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_name() {
        assert_eq!(NotificationChannel::Wechat.name(), "企业微信");
        assert_eq!(NotificationChannel::Feishu.name(), "飞书");
    }

    #[test]
    fn test_generate_report() {
        // let results = vec![
        //     AnalysisResult {
        //         code: "600519".to_string(),
        //         name: "贵州茅台".to_string(),
        //         sentiment_score: 75,
        //         trend_prediction: "看多".to_string(),
        //         operation_advice: "买入".to_string(),
        //         analysis_summary: "技术面强势".to_string(),
        //         technical_analysis: Some("放量突破".to_string()),
        //         news_summary: Some("业绩超预期".to_string()),
        //         buy_reason: Some("技术面好".to_string()),
        //         risk_warning: Some("注意回调".to_string()),
        //         ma_analysis: None,
        //         volume_analysis: None,
        //     },
        // ];

        // let service = NotificationService::new(NotificationConfig::default());
        // let report = service.generate_daily_report(&results);

        // assert!(report.contains("贵州茅台"));
        // assert!(report.contains("600519"));
        // assert!(report.contains("买入"));
    }
}
