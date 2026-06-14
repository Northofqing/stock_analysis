//! 盘前 Checklist 与收盘总结生成。
//!
//! 盘前 08:45: 外盘环境 + 今日大事 + T+1解禁预警 + 持仓风险
//! 收盘 15:30: 市场概况 + 操作回顾 + 信号统计 + 明日预判

use crate::calendar::{self, MarketSession};
use chrono::Local;

/// 盘前 Checklist
pub fn build_pre_market_checklist(
    positions: &[PositionSummary],
    t1_unlocks: &[PositionSummary],
    macro_events: &[String],
) -> String {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let next_trade = calendar::next_trading_day(Local::now().date_naive());
    let mut lines = vec![
        format!("# 📋 今日开盘 Checklist（{}）", today),
        String::new(),
    ];

    // 外盘环境
    lines.push("## 🌍 外盘环境".into());
    lines.push("（数据源：金十快讯 + 华尔街见闻，08:45自动刷新）".into());
    if macro_events.is_empty() {
        lines.push("- 暂无重大宏观事件".into());
    } else {
        for e in macro_events.iter().take(5) {
            lines.push(format!("- {}", e));
        }
    }
    lines.push(String::new());

    // 持仓风险检查
    if !positions.is_empty() {
        lines.push("## 💰 持仓风险检查".into());
        lines.push("| 代码 | 名称 | 现价 | 止损价 | 距离 | 状态 |".into());
        lines.push("|------|------|------|--------|------|------|".into());
        for p in positions {
            let dist = if p.stop_loss > 0.0 {
                format!("{:.1}%", (p.current_price - p.stop_loss) / p.current_price * 100.0)
            } else { "-".into() };
            let status = if p.t1_locked { "🔒 T+1" } else { "✅ 可用" };
            lines.push(format!(
                "| {} | {} | {:.2} | {:.2} | {} | {} |",
                p.code, p.name, p.current_price, p.stop_loss, dist, status
            ));
        }
        lines.push(String::new());
    }

    // T+1 解禁预警
    if !t1_unlocks.is_empty() {
        lines.push("## ⚠️ T+1 解禁预警（今日可卖）".into());
        for p in t1_unlocks {
            lines.push(format!(
                "- **{}({})** 现价 {:.2}，止损 {:.2}。今日竞价关注，如低开 >2% 建议挂单",
                p.name, p.code, p.current_price, p.stop_loss
            ));
        }
        lines.push(String::new());
    }

    // 今日大事
    lines.push("## 📅 今日关注".into());
    lines.push(format!("- 下一个交易日：{}", next_trade.format("%Y-%m-%d")));
    lines.push("- 09:25 集合竞价结果将自动修正本 Checklist".into());
    lines.push(String::new());

    lines.push("---".into());
    lines.push("*本 Checklist 由监控模块自动生成，竞价后更新*".into());
    lines.join("\n")
}

/// 收盘总结
pub fn build_close_summary(
    market_pct: f64,
    limit_up_count: usize,
    limit_down_count: usize,
    break_rate: f64,
    signals_today: usize,
    alerts_sent: usize,
    t1_holdings: &[PositionSummary],
) -> String {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut lines = vec![
        format!("# 📊 今日复盘（{}）", today),
        String::new(),
        "## 📈 市场概况".into(),
        format!("- 沪指：{:+.2}%", market_pct),
        format!("- 涨停 {} 只 | 跌停 {} 只 | 炸板率 {:.0}%", limit_up_count, limit_down_count, break_rate),
        String::new(),
        "## 📡 今日信号统计".into(),
        format!("- 触发信号：{} 条", signals_today),
        format!("- 推送告警：{} 条", alerts_sent),
        String::new(),
    ];

    if !t1_holdings.is_empty() {
        lines.push("## ⚠️ T+1 冻结股明日预警".into());
        for p in t1_holdings {
            lines.push(format!(
                "- **{}({})** 现价 {:.2}，止损 {:.2}。明日解禁，竞价关注",
                p.name, p.code, p.current_price, p.stop_loss
            ));
        }
        lines.push(String::new());
    }

    lines.push("---".into());
    lines.push("*收盘总结由监控模块自动生成*".into());
    lines.join("\n")
}

/// 持仓摘要（用于 Checklist 和收盘总结）
#[derive(Debug, Clone)]
pub struct PositionSummary {
    pub code: String,
    pub name: String,
    pub current_price: f64,
    pub stop_loss: f64,
    pub t1_locked: bool,
}

impl PositionSummary {
    pub fn from_db() -> Vec<Self> {
        let db = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            Ok(db) => db,
            Err(_) => return Vec::new(),
        };
        match db.get_all_open_positions() {
            Ok(positions) => positions
                .iter()
                .map(|p| PositionSummary {
                    code: p.code.clone(),
                    name: p.name.clone(),
                    current_price: 0.0,
                    stop_loss: 0.0,
                    t1_locked: false,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// 获取当前时段该做什么
pub fn session_action() -> &'static str {
    match calendar::current_session() {
        MarketSession::Closed => "休市，无需操作",
        MarketSession::Auction => "09:25 竞价扫描中，关注竞价异动",
        MarketSession::Morning => "盘中监控中（上午盘）",
        MarketSession::LunchBreak => "午休，暂停扫描",
        MarketSession::Afternoon => "盘中监控中（下午盘）",
        MarketSession::AfterHours => "收盘，生成复盘总结",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pre_market_empty() {
        let text = build_pre_market_checklist(&[], &[], &[]);
        assert!(text.contains("开盘 Checklist"));
        assert!(text.contains("暂无重大宏观事件"));
    }

    #[test]
    fn test_pre_market_with_positions() {
        let positions = vec![PositionSummary {
            code: "000001".into(), name: "测试".into(),
            current_price: 10.0, stop_loss: 9.2, t1_locked: false,
        }];
        let text = build_pre_market_checklist(&positions, &[], &[]);
        assert!(text.contains("000001"));
        assert!(text.contains("10.00"));
        assert!(text.contains("9.20"));
    }

    #[test]
    fn test_pre_market_with_t1_unlocks() {
        let t1 = vec![PositionSummary {
            code: "000002".into(), name: "解禁股".into(),
            current_price: 10.0, stop_loss: 9.0, t1_locked: true,
        }];
        let text = build_pre_market_checklist(&[], &t1, &[]);
        assert!(text.contains("T+1 解禁预警"));
        assert!(text.contains("解禁股"));
    }

    #[test]
    fn test_close_summary() {
        let text = build_close_summary(0.82, 94, 3, 28.0, 12, 3, &[]);
        assert!(text.contains("今日复盘"));
        assert!(text.contains("+0.82%"));
        assert!(text.contains("94"));
    }

    #[test]
    fn test_close_summary_with_t1() {
        let t1 = vec![PositionSummary {
            code: "000001".into(), name: "冻结股".into(),
            current_price: 10.0, stop_loss: 9.0, t1_locked: true,
        }];
        let text = build_close_summary(0.0, 0, 0, 0.0, 0, 0, &t1);
        assert!(text.contains("T+1 冻结股明日预警"));
    }
}
