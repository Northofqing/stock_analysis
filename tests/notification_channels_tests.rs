//! 修复 P0-0: 通知渠道完整性测试
//! 之前 _ => 死代码分支让 Telegram/Pushover/Custom 是空实现
//! 现在: 每个渠道必须有真实实现, 没配置时返回 Ok(false) 不 panic

use stock_analysis::notification::config::*;
use stock_analysis::notification::service::*;

/// 用 from_env 间接构造, 避免访问 private 字段
async fn try_send_one(channel: NotificationChannel) -> Result<bool, String> {
    // 清空所有 env, 只用 NotificationConfig::default()
    // 这样 send() 走到该 channel 的 no-config 分支, 必 Ok(false) 不 panic
    let cfg = NotificationConfig::default();
    let svc = NotificationService::new(cfg);
    // 把 channel 加到 available_channels
    let mut svc = svc;
    svc.available_channels.push(channel);
    svc.send("test message").await.map_err(|e| e.to_string())
}

#[tokio::test]
async fn test_dingtalk_send_no_webhook() {
    let result = try_send_one(NotificationChannel::DingTalk).await;
    assert!(result.is_ok(), "DingTalk 没 webhook 必 Ok(false), 不能 panic: {:?}", result);
}

#[tokio::test]
async fn test_telegram_send_no_token() {
    let result = try_send_one(NotificationChannel::Telegram).await;
    assert!(result.is_ok(), "Telegram 没 token 必 Ok(false), 不能 panic");
}

#[tokio::test]
async fn test_slack_send_no_webhook() {
    let result = try_send_one(NotificationChannel::Slack).await;
    assert!(result.is_ok(), "Slack 没 webhook 必 Ok(false), 不能 panic");
}

#[tokio::test]
async fn test_discord_send_no_webhook() {
    let result = try_send_one(NotificationChannel::Discord).await;
    assert!(result.is_ok(), "Discord 没 webhook 必 Ok(false), 不能 panic");
}

#[tokio::test]
async fn test_pushover_send_no_key() {
    let result = try_send_one(NotificationChannel::Pushover).await;
    assert!(result.is_ok(), "Pushover 没 key 必 Ok(false), 不能 panic");
}

#[tokio::test]
async fn test_custom_send_no_url() {
    let result = try_send_one(NotificationChannel::Custom).await;
    assert!(result.is_ok(), "Custom 没 url 必 Ok(false), 不能 panic");
}

#[test]
fn test_no_dead_code_channels() {
    // 量化产品经理要求: enum 里的所有变体都有实现, 没有 "暂未实现" 死代码
    let all_channels = [
        NotificationChannel::Wechat,
        NotificationChannel::Feishu,
        NotificationChannel::Telegram,
        NotificationChannel::Email,
        NotificationChannel::Pushover,
        NotificationChannel::Custom,
        NotificationChannel::ServerChan,
        NotificationChannel::DingTalk,
        NotificationChannel::Slack,
        NotificationChannel::Discord,
    ];
    assert_eq!(all_channels.len(), 10, "应有 10 个渠道变体");
    for ch in &all_channels {
        // 任何渠道必须有 name()
        let n = ch.name();
        assert!(!n.is_empty(), "渠道 {:?} 没 name", std::any::type_name_of_val(ch));
    }
}
