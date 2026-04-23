//! 通知服务主入口
//!
//! 仅含 `NotificationService` 结构与"生命周期 + 统一发送 + 文件保存"方法，
//! 具体渠道实现（微信 / 飞书 / 邮件 / 报告生成）位于同级子模块。

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use log::{error, info, warn};
use reqwest::Client;

use super::config::{NotificationChannel, NotificationConfig};

/// 股票分析结果（与 `pipeline::AnalysisResult` 共用同一类型，避免重复定义）。
pub use crate::pipeline::AnalysisResult;

/// 通知服务
pub struct NotificationService {
    pub(super) config: NotificationConfig,
    pub(super) client: Client,
    pub(super) available_channels: Vec<NotificationChannel>,
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

}

impl NotificationService {
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
