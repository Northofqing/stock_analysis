//! 板块退潮退出 — 连续资金流出检测。

#[derive(Debug, Clone)]
pub struct SectorExitSignal {
    pub sector_name: String,
    pub consecutive_weeks_outflow: u32,
    pub should_exit: bool,
    pub reason: String,
}

/// 硬约束：连续 N 周主力资金净流出 → 触发退出信号
pub fn check_sector_exit(
    sector_name: &str,
    weekly_flows: &[f64], // 最近 N 周的主力净流入（正=流入，负=流出）
    threshold_weeks: u32,
) -> Option<SectorExitSignal> {
    if weekly_flows.is_empty() {
        return None;
    }

    // 从最近一周往前数，连续净流出的周数
    let mut consecutive = 0u32;
    for flow in weekly_flows.iter().rev() {
        if *flow < 0.0 {
            consecutive += 1;
        } else {
            break;
        }
    }

    if consecutive >= threshold_weeks {
        Some(SectorExitSignal {
            sector_name: sector_name.to_string(),
            consecutive_weeks_outflow: consecutive,
            should_exit: true,
            reason: format!("连续 {} 周主力净流出", consecutive),
        })
    } else {
        None
    }
}

/// 格式化板块退潮告警
pub fn format_sector_exit(signals: &[SectorExitSignal]) -> String {
    if signals.is_empty() {
        return String::new();
    }
    let mut lines = vec!["🔻 板块退潮预警".to_string()];
    for s in signals {
        lines.push(format!(
            "  ⚠️ {} — {}，建议减仓并移出观察池",
            s.sector_name, s.reason,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_three_weeks_outflow_triggers_exit() {
        // 从旧到新排列：[-3e8, -2e8, -1e8] → 最近3周全部净流出
        let flows = vec![-3e8, -2e8, -1e8];
        let signal = check_sector_exit("测试板块", &flows, 3).unwrap();
        assert!(signal.should_exit);
        assert_eq!(signal.consecutive_weeks_outflow, 3);
    }

    #[test]
    fn test_two_weeks_no_trigger() {
        let flows = vec![-1e8, -2e8, 5e7]; // 仅2周净流出
        assert!(check_sector_exit("测试板块", &flows, 3).is_none());
    }

    #[test]
    fn test_empty_flow() {
        assert!(check_sector_exit("测试板块", &[], 3).is_none());
    }

    #[test]
    fn test_recent_inflow_breaks_sequence_and_alert_formatting() {
        assert!(check_sector_exit("测试板块", &[-3.0, -2.0, 1.0, -1.0], 2).is_none());
        assert_eq!(format_sector_exit(&[]), "");

        let signal = check_sector_exit("测试板块", &[-3.0, -2.0], 2).unwrap();
        let alert = format_sector_exit(&[signal]);
        assert!(alert.starts_with("🔻 板块退潮预警"));
        assert!(alert.contains("测试板块"));
        assert!(alert.contains("连续 2 周主力净流出"));
    }
}
