//! 通知渠道 / SMTP / 顶层配置（原 notification.rs 前部）


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

    /// 校验各渠道配置的一致性。
    ///
    /// 规则：任一渠道若部分字段有值则必须全部配齐；否则视作未启用。
    /// 返回错误描述列表，空表示全部合法。
    pub fn validate(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // --- 邮件（5 项必须同时配置）---
        let email_fields: [(&str, bool); 5] = [
            ("EMAIL_SENDER", self.email_sender.is_some()),
            ("EMAIL_PASSWORD", self.email_password.is_some()),
            ("EMAIL_RECEIVERS", !self.email_receivers.is_empty()),
            ("SMTP_SERVER", self.smtp_server.is_some()),
            ("SMTP_PORT", self.smtp_port.is_some()),
        ];
        let email_set = email_fields.iter().filter(|(_, v)| *v).count();
        if email_set > 0 && email_set < email_fields.len() {
            let missing: Vec<&str> = email_fields
                .iter()
                .filter(|(_, v)| !*v)
                .map(|(k, _)| *k)
                .collect();
            errors.push(format!(
                "邮件通知配置不完整，缺少: {}（请在 .env 中补齐，或删除全部邮件相关配置以禁用邮件）",
                missing.join(", ")
            ));
        }

        // SMTP_PORT 若 env 有值但解析失败 → 报错
        if let Ok(raw) = std::env::var("SMTP_PORT") {
            if !raw.trim().is_empty() && self.smtp_port.is_none() {
                errors.push(format!(
                    "SMTP_PORT 配置无效: \"{}\"（必须是 1-65535 的整数，如 465 或 587）",
                    raw
                ));
            }
        }

        // --- Telegram（两项必须同时）---
        let tg_token = self.telegram_bot_token.is_some();
        let tg_chat = self.telegram_chat_id.is_some();
        if tg_token ^ tg_chat {
            let miss = if tg_token {
                "TELEGRAM_CHAT_ID"
            } else {
                "TELEGRAM_BOT_TOKEN"
            };
            errors.push(format!("Telegram 通知配置不完整，缺少: {}", miss));
        }

        // --- Pushover（两项必须同时）---
        let po_user = self.pushover_user_key.is_some();
        let po_tok = self.pushover_api_token.is_some();
        if po_user ^ po_tok {
            let miss = if po_user {
                "PUSHOVER_API_TOKEN"
            } else {
                "PUSHOVER_USER_KEY"
            };
            errors.push(format!("Pushover 通知配置不完整，缺少: {}", miss));
        }

        errors
    }
}

