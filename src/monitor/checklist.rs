//! 盘前 Checklist 与收盘总结生成。
//!
//! 盘前 08:45: 外盘环境 + 今日大事 + T+1解禁预警 + 持仓风险
//! 收盘 15:30: 市场概况 + 操作回顾 + 信号统计 + 明日预判

use crate::calendar::{self, MarketSession};
use crate::portfolio::Position;
use chrono::Local;

/// 盘前 Checklist
pub fn build_pre_market_checklist(
    positions: &[Position],
    t1_unlocks: &[Position],
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
        lines.push("| 代码 | 名称 | 成本 | 止损 | 距离 | 股数 | 状态 |".into());
        lines.push("|------|------|------|------|------|------|------|".into());
        for p in positions {
            let dist = if p.hard_stop > 0.0 && p.cost_price > 0.0 {
                format!("{:.1}%", (p.cost_price - p.hard_stop) / p.cost_price * 100.0)
            } else { "-".into() };
            // review #14: DB 失败时不再静默 "可用", 而是显示警告, 让 operator 知道有异常.
            let status = match crate::portfolio::is_t1_locked(&p.code) {
                Ok(true) => "🔒 T+1",
                Ok(false) => "✅ 可用",
                Err(_) => "⚠️ DB",
            };
            lines.push(format!(
                "| {} | {} | {:.2} | {:.2} | {} | {} | {} |",
                p.code, p.name, p.cost_price, p.hard_stop, dist, p.shares, status
            ));
        }
        lines.push(String::new());
    }

    // T+1 解禁预警
    if !t1_unlocks.is_empty() {
        lines.push("## ⚠️ T+1 解禁预警（今日可卖）".into());
        for p in t1_unlocks {
            lines.push(format!(
                "- **{}({})** 成本 {:.2}，止损 {:.2}。今日竞价关注，如低开 >2% 建议挂单",
                p.name, p.code, p.cost_price, p.hard_stop
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
///
/// v13.10.1 P0-#5: 区分两种语义, 不再混用:
/// - `t1_frozen`: 今日买入, 明日解禁可卖 (持仓快照, 标 "T+1 冻结")
/// - `tomorrow_unlocks`: 今日解禁, 明日可卖 (现持仓, 标 "明日可卖")
/// 同时止损为 0 时不写 "止损 0.00" 误导.
pub fn build_close_summary(
    market_pct: f64,
    limit_up_count: usize,
    limit_down_count: usize,
    break_rate: f64,
    signals_today: usize,
    alerts_sent: usize,
    t1_frozen: &[Position],
    tomorrow_unlocks: &[Position],
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

    if !t1_frozen.is_empty() {
        lines.push("## 🔒 T+1 冻结股（明日解禁可卖）".into());
        for p in t1_frozen {
            let stop = if p.hard_stop > 0.0 {
                format!("止损 {:.2}", p.hard_stop)
            } else {
                "止损未设".to_string()
            };
            lines.push(format!(
                "- **{}({})** 成本 {:.2}, {}. 明日解禁, 竞价关注",
                p.name, p.code, p.cost_price, stop
            ));
        }
        lines.push(String::new());
    }

    if !tomorrow_unlocks.is_empty() {
        lines.push("## ✅ 明日可卖（今日已解禁）".into());
        for p in tomorrow_unlocks {
            let stop = if p.hard_stop > 0.0 {
                format!("止损 {:.2}", p.hard_stop)
            } else {
                "止损未设".to_string()
            };
            lines.push(format!(
                "- **{}({})** 成本 {:.2}, {}. 明日可自由买卖",
                p.name, p.code, p.cost_price, stop
            ));
        }
        lines.push(String::new());
    }

    lines.push("---".into());
    lines.push("*收盘总结由监控模块自动生成*".into());
    lines.join("\n")
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
    use chrono::NaiveDate;

    fn init_db() {
        let _ = crate::database::DatabaseManager::init(Some(std::path::PathBuf::from("./test_data/test.db")));
    }

    fn pos(code: &str, name: &str, cost: f64, stop: f64) -> Position {
        Position {
            code: code.into(), name: name.into(), shares: 1000,
            cost_price: cost, hard_stop: stop,
            added_at: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: "其他".into(), ..Default::default()
        }
    }

    #[test]
    fn test_pre_market_empty() {
        let text = build_pre_market_checklist(&[], &[], &[]);
        assert!(text.contains("开盘 Checklist"));
        assert!(text.contains("暂无重大宏观事件"));
    }

    #[test]
    fn test_pre_market_with_positions() {
        init_db();
        let positions = vec![pos("000001", "测试", 10.0, 9.2)];
        let text = build_pre_market_checklist(&positions, &[], &[]);
        assert!(text.contains("000001"));
        assert!(text.contains("10.00"));
        assert!(text.contains("9.20"));
    }

    #[test]
    fn test_pre_market_with_t1_unlocks() {
        init_db();
        let t1 = vec![pos("000002", "解禁股", 10.0, 9.0)];
        let text = build_pre_market_checklist(&[], &t1, &[]);
        assert!(text.contains("T+1 解禁预警"));
        assert!(text.contains("解禁股"));
    }

    #[test]
    fn test_close_summary() {
        let text = build_close_summary(0.82, 94, 3, 28.0, 12, 3, &[], &[]);
        assert!(text.contains("今日复盘"));
        assert!(text.contains("+0.82%"));
        assert!(text.contains("94"));
    }

    #[test]
    fn test_close_summary_with_t1_frozen() {
        init_db();
        let t1 = vec![pos("000001", "冻结股", 10.0, 9.0)];
        let text = build_close_summary(0.0, 0, 0, 0.0, 0, 0, &t1, &[]);
        assert!(text.contains("T+1 冻结股"));
        assert!(text.contains("明日解禁"));
    }

    #[test]
    fn test_close_summary_with_tomorrow_unlocks() {
        init_db();
        let unlocks = vec![pos("000002", "可卖", 10.0, 9.0)];
        let text = build_close_summary(0.0, 0, 0, 0.0, 0, 0, &[], &unlocks);
        assert!(text.contains("明日可卖"));
        assert!(text.contains("可自由买卖"));
    }

    #[test]
    fn test_close_summary_hides_zero_stop() {
        // 止损为 0 时不应显示 "止损 0.00"
        let t1 = vec![pos("000003", "无止损", 10.0, 0.0)];
        let text = build_close_summary(0.0, 0, 0, 0.0, 0, 0, &t1, &[]);
        assert!(text.contains("止损未设"));
        assert!(!text.contains("止损 0.00"));
    }
}
