//! v12 MVP-4 §7.3: 涨停产业链复盘 (R-03 推送).
//!
//! 设计: 主线/龙头/后排/退潮警示 (阶段字段用 HeatStage).

use crate::opportunity::news_ranker::HeatStage;

/// 涨停产业链复盘条目
#[derive(Debug, Clone)]
pub struct ChainLimitUpItem {
    pub chain: String,
    pub limit_up_count: usize,
    pub first_limit_up: usize, // 首板数
    pub continuous: usize,     // 连板数
    pub leader_name: String,
    pub leader_board_count: usize, // 龙头板数
    pub stage: HeatStage,
    pub watch_point: String,
}

/// 退潮警示 (前一日涨停数 ≥ 3, 当日 ≥ 1 但 ≤ 3, 资金净流出)
#[derive(Debug, Clone)]
pub struct FadeWarning {
    pub chain: String,
    pub prev_limit_up: usize,
    pub today_limit_up: usize,
    pub main_outflow: f64,
}

/// R-03 复盘渲染
pub fn render_r03(items: &[ChainLimitUpItem], fade: &[FadeWarning]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "🔥 涨停产业链（{}）\n",
        chrono::Local::now().format("%Y-%m-%d")
    ));
    for (i, it) in items.iter().enumerate() {
        s.push_str(&format!(
            "{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {} {}板\n   明日观察: {}\n",
            i + 1,
            it.chain,
            it.limit_up_count,
            it.first_limit_up,
            it.continuous,
            it.stage.label(),
            it.leader_name,
            it.leader_board_count,
            it.watch_point,
        ));
    }
    if !fade.is_empty() {
        s.push_str("⚠️ 退潮链:\n");
        for f in fade {
            s.push_str(&format!(
                "{}（涨停{}→{}家, 资金流出{:.0}万）暂回避\n",
                f.chain,
                f.prev_limit_up,
                f.today_limit_up,
                f.main_outflow / 1e4,
            ));
        }
    }
    s
}

/// 输入: 板块名 + 涨停数 + 龙头信息 + 资金 → ChainLimitUpItem
pub fn build_chain_item(
    chain: String,
    limit_up_count: usize,
    first_limit_up: usize,
    continuous: usize,
    leader_name: String,
    leader_board_count: usize,
    main_inflow: f64,
) -> ChainLimitUpItem {
    let stage = if limit_up_count >= 10 {
        HeatStage::Climax
    } else if limit_up_count >= 5 {
        HeatStage::Ferment
    } else if limit_up_count >= 1 {
        HeatStage::Start
    } else {
        HeatStage::Cold
    };
    let watch_point = if main_inflow > 1e8 {
        "资金大幅流入, 龙头可能加速"
    } else if main_inflow > 0.0 {
        "资金小幅流入, 关注持续性"
    } else {
        "资金流出, 谨防分歧"
    };
    ChainLimitUpItem {
        chain,
        limit_up_count,
        first_limit_up,
        continuous,
        leader_name,
        leader_board_count,
        stage,
        watch_point: watch_point.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_basic() {
        let items = vec![build_chain_item(
            "机器人".into(),
            8,
            3,
            5,
            "XX科技".into(),
            5,
            5e7,
        )];
        let s = render_r03(&items, &[]);
        assert!(s.contains("机器人"));
        assert!(s.contains("涨停8家"));
        assert!(s.contains("XX科技"));
    }

    #[test]
    fn render_with_fade() {
        let items = vec![build_chain_item(
            "AI硬件".into(),
            3,
            2,
            1,
            "YY".into(),
            1,
            -3e7,
        )];
        let fade = vec![FadeWarning {
            chain: "元宇宙".into(),
            prev_limit_up: 5,
            today_limit_up: 1,
            main_outflow: -8e7,
        }];
        let s = render_r03(&items, &fade);
        assert!(s.contains("退潮链"));
        assert!(s.contains("元宇宙"));
    }

    #[test]
    fn stage_climax_high_count() {
        let it = build_chain_item("X".into(), 12, 5, 7, "Y".into(), 7, 1e8);
        assert_eq!(it.stage, HeatStage::Climax);
    }

    #[test]
    fn stage_start_low_count() {
        let it = build_chain_item("X".into(), 2, 2, 0, "Y".into(), 1, 1e7);
        assert_eq!(it.stage, HeatStage::Start);
    }

    #[test]
    fn stage_cold_zero_count() {
        let it = build_chain_item("X".into(), 0, 0, 0, "Y".into(), 0, 0.0);
        assert_eq!(it.stage, HeatStage::Cold);
    }
}
