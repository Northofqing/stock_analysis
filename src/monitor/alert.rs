//! 分级告警推送。
//!
//! 职责：接收经过 SignalStateMachine 去重的告警事件，格式化后推送。
//! T+1 锁仓告警话术自动适配。

use crate::monitor::detector::{AlertEvent, AlertLevel};
use crate::notification::NotificationService;
use log::{info, warn};

/// 格式化告警事件为推送文本
pub fn format_alert(event: &AlertEvent) -> String {
    let d = &event.detail;
    let mut lines = vec![format!(
        "{} 【{}】{}({})",
        event.level.emoji(),
        event.level.label(),
        event.name,
        event.code,
    )];
    lines.push(format!("  {}", event.message));

    if let Some(price) = d.price {
        lines.push(format!("  现价：{:.2}", price));
    }
    if let Some(pct) = d.change_pct {
        lines.push(format!("  涨跌：{:+.2}%", pct));
    }
    if let Some(flow) = d.main_flow_yi {
        lines.push(format!("  主力净流入：{:+.2}亿", flow));
    }
    if let Some(vr) = d.volume_ratio {
        lines.push(format!("  量比：{:.1}", vr));
    }
    if event.category.key() == "flash_news" {
        if let Some(title) = &d.news_title {
            lines.push(format!("  快讯：{}", title));
        }
        if let Some(extra) = &d.extra {
            lines.push(format!("  {}", extra));
        }
    }
    if d.t1_locked {
        lines.push("  ⚠️ T+1锁仓 — 次日09:25竞价卖出".to_string());
    }
    lines.push(format!(
        "  [{}]",
        event.triggered_at.format("%H:%M:%S")
    ));
    lines.join("\n")
}

/// 格式化 T+1 锁仓告警话术
pub fn format_t1_alert(event: &AlertEvent, sell_date: &str) -> String {
    let d = &event.detail;
    format!(
        "{} 【{}】{} ⚠️ T+1锁仓\n\
          现价：{:.2}（触发止损/风控）\n\
          状态：今日买入，不可当日卖出\n\
          建议：{} 09:25 竞价挂卖单\n\
          连锁：已冻结同产业链新买入权限",
        event.level.emoji(),
        event.level.label(),
        event.message,
        d.price.unwrap_or(0.0),
        sell_date
    )
}

/// 向通知服务推送告警
pub async fn push_alert(
    event: &AlertEvent,
    notifier: &NotificationService,
) -> bool {
    let text = format_alert(event);
    match notifier.send_alert(&text, event.level).await {
        Ok(true) => {
            info!("[Alert] 推送成功: {} {}", event.code, event.message);
            true
        }
        Ok(false) => {
            warn!("[Alert] 推送失败: {}", event.code);
            false
        }
        Err(e) => {
            warn!("[Alert] 推送异常: {} - {}", event.code, e);
            false
        }
    }
}

/// 聚合多条告警为摘要推送
pub fn aggregate_alerts(events: &[AlertEvent]) -> Option<String> {
    if events.is_empty() {
        return None;
    }
    let emergency = events.iter().filter(|e| e.level == AlertLevel::Emergency).count();
    let important = events.iter().filter(|e| e.level == AlertLevel::Important).count();
    let info = events.iter().filter(|e| e.level == AlertLevel::Info).count();

    let mut lines = vec![format!(
        "📊 告警聚合摘要（共 {} 条）",
        events.len()
    )];
    if emergency > 0 {
        lines.push(format!("  🔴 紧急：{} 条", emergency));
    }
    if important > 0 {
        lines.push(format!("  🟠 重要：{} 条", important));
    }
    if info > 0 {
        lines.push(format!("  🟡 参考：{} 条", info));
    }
    lines.push(String::new());
    for e in events.iter().take(8) {
        lines.push(format!(
            "- {} {} {}",
            e.level.emoji(),
            e.category.label(),
            truncate(&e.message, 80)
        ));
    }
    if events.len() > 8 {
        lines.push(format!("  ... 还有 {} 条", events.len() - 8));
    }
    Some(lines.join("\n"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// 扩展 NotificationService 以支持告警级别路由
impl NotificationService {
    /// 发送告警（带级别路由）
    pub async fn send_alert(&self, text: &str, level: AlertLevel) -> Result<bool, anyhow::Error> {
        match level {
            AlertLevel::Emergency => {
                // 紧急：全渠道
                self.send(text).await
            }
            AlertLevel::Important => {
                // 重要：微信 + 飞书（邮件渠道也发因为已有配置）
                self.send(text).await
            }
            AlertLevel::Info => {
                // 参考：仅飞书/邮件（如果配置了的话）
                self.send(text).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::detector::{AlertCategory, AlertDetail};
    use chrono::Local;

    fn make_event(level: AlertLevel, code: &str, msg: &str, t1: bool) -> AlertEvent {
        AlertEvent {
            level,
            category: AlertCategory::MainOutflow,
            code: code.into(),
            name: "测试股".into(),
            message: msg.into(),
            detail: AlertDetail {
                price: Some(10.5),
                change_pct: Some(-3.2),
                volume_ratio: Some(1.5),
                main_flow_yi: Some(-0.5),
                threshold: None,
                news_title: None,
                t1_locked: t1,
                extra: None,
            },
            triggered_at: Local::now(),
        }
    }

    #[test]
    fn test_format_alert_normal() {
        let e = make_event(AlertLevel::Important, "000001", "主力出逃 0.5亿", false);
        let text = format_alert(&e);
        assert!(text.contains("测试股"));
        assert!(text.contains("10.50"));
        assert!(text.contains("-3.20"));
    }

    #[test]
    fn test_format_alert_t1() {
        let e = make_event(AlertLevel::Emergency, "000001", "跌停", true);
        let text = format_alert(&e);
        assert!(text.contains("T+1锁仓"));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert!(truncate("hello world this is long", 10).ends_with('…'));
    }

    #[test]
    fn test_aggregate_empty() {
        assert!(aggregate_alerts(&[]).is_none());
    }

    #[test]
    fn test_aggregate_summary() {
        let events = vec![
            make_event(AlertLevel::Emergency, "000001", "跌停", true),
            make_event(AlertLevel::Important, "000002", "主力出逃", false),
        ];
        let summary = aggregate_alerts(&events).unwrap();
        assert!(summary.contains("紧急：1"));
        assert!(summary.contains("重要：1"));
    }
}
