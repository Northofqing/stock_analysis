//! 赛道分档引擎 — 五维分档（强/中/弱），输出第一/二/三梯队。

use crate::market_analyzer::sector_monitor::ConceptBoard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Strong,
    Medium,
    Weak,
}

impl Grade {
    pub fn label(&self) -> &'static str {
        match self {
            Grade::Strong => "强",
            Grade::Medium => "中",
            Grade::Weak => "弱",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorTier {
    Tier1,
    Tier2,
    Watch,
    Excluded,
}

impl SectorTier {
    pub fn label(&self) -> &'static str {
        match self {
            SectorTier::Tier1 => "第一梯队·核心主线",
            SectorTier::Tier2 => "第二梯队·卫星弹性",
            SectorTier::Watch => "观察区·不入场",
            SectorTier::Excluded => "排除清单·不关注",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GradedSector {
    pub name: String,
    pub code: String,
    pub change_pct: f64,
    pub main_inflow: f64,
    pub capital_grade: Grade,
    pub technical_grade: Grade,
    pub tier: SectorTier,
}

/// 对板块排名做分档。policy 和 industry 维度暂用中性（需要额外数据源），
/// capital 和 technical 从 sector_monitor 实时数据判定。
pub fn grade_sectors(boards: &[ConceptBoard]) -> Vec<GradedSector> {
    if boards.is_empty() {
        return vec![];
    }

    // 计算中位数做相对比较基准
    let median_change = median(
        boards
            .iter()
            .map(|b| b.change_pct)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let median_inflow = median(
        boards
            .iter()
            .map(|b| b.main_inflow)
            .collect::<Vec<_>>()
            .as_slice(),
    );

    boards
        .iter()
        .map(|b| {
            // 资金维度：正流入且远超中位数 → 强，正流入 → 中，净流出 → 弱
            let capital = if b.main_inflow > 0.0 && b.main_inflow > median_inflow * 2.0 {
                Grade::Strong
            } else if b.main_inflow > 0.0 {
                Grade::Medium
            } else {
                Grade::Weak
            };

            // 技术维度：正涨幅且远超中位数 → 强，正涨幅 → 中，下跌 → 弱
            let technical = if b.change_pct > 0.0 && b.change_pct > median_change * 2.0 {
                Grade::Strong
            } else if b.change_pct > 0.0 {
                Grade::Medium
            } else {
                Grade::Weak
            };

            // 分档：资本+技术，加权简化为 ≥2强 → Tier1, 1强 → Tier2
            let strong_count = [capital, technical]
                .iter()
                .filter(|g| **g == Grade::Strong)
                .count();
            let weak_count = [capital, technical]
                .iter()
                .filter(|g| **g == Grade::Weak)
                .count();

            let tier = if weak_count >= 2 {
                SectorTier::Watch
            } else if strong_count >= 2 {
                SectorTier::Tier1
            } else if strong_count == 1 {
                SectorTier::Tier2
            } else {
                SectorTier::Watch
            };

            GradedSector {
                name: b.name.clone(),
                code: b.code.clone(),
                change_pct: b.change_pct,
                main_inflow: b.main_inflow,
                capital_grade: capital,
                technical_grade: technical,
                tier,
            }
        })
        .collect()
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// 格式化梯队榜单
pub fn format_tier_list(sectors: &[GradedSector]) -> String {
    if sectors.is_empty() {
        return "无板块数据".to_string();
    }

    let tier1: Vec<_> = sectors
        .iter()
        .filter(|s| s.tier == SectorTier::Tier1)
        .collect();
    let tier2: Vec<_> = sectors
        .iter()
        .filter(|s| s.tier == SectorTier::Tier2)
        .collect();

    let mut lines = vec![format!(
        "📊 赛道分档（{}）",
        chrono::Local::now().format("%H:%M")
    )];
    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());

    if !tier1.is_empty() {
        lines.push("🥇 第一梯队·核心主线".to_string());
        for s in tier1.iter().take(3) {
            lines.push(format!(
                "  {} {:+.1}% 资金{} 技术{}",
                s.name,
                s.change_pct,
                s.capital_grade.label(),
                s.technical_grade.label()
            ));
        }
    }

    if !tier2.is_empty() {
        lines.push(String::new());
        lines.push("🥈 第二梯队·卫星弹性".to_string());
        for s in tier2.iter().take(5) {
            lines.push(format!(
                "  {} {:+.1}% 资金{} 技术{}",
                s.name,
                s.change_pct,
                s.capital_grade.label(),
                s.technical_grade.label()
            ));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(name: &str, change: f64, inflow: f64) -> ConceptBoard {
        ConceptBoard {
            code: format!("BK{}", name),
            name: name.to_string(),
            change_pct: change,
            main_inflow: inflow,
            leader_name: String::new(),
            vol_ratio: 0.0,
            turnover: 0.0,
            main_net_pct_today: 0.0,
            main_net_pct_5d: 0.0,
        }
    }

    #[test]
    fn test_median() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    }

    #[test]
    fn test_all_watch() {
        let boards = vec![board("A", -5.0, -1e8), board("B", -3.0, -5e7)];
        let graded = grade_sectors(&boards);
        assert!(graded.iter().all(|g| g.tier == SectorTier::Watch));
    }

    #[test]
    fn test_strong_rises_to_tier1() {
        let boards = vec![
            board("A", 8.0, 10e8), // strong tech + strong capital
            board("B", 1.0, 1e8),
            board("C", 0.5, 0.5e8),
        ];
        let graded = grade_sectors(&boards);
        let tier1: Vec<_> = graded
            .iter()
            .filter(|g| g.tier == SectorTier::Tier1)
            .collect();
        assert!(!tier1.is_empty());
    }

    #[test]
    fn labels_empty_input_and_tier_rendering_cover_all_public_states() {
        assert_eq!(Grade::Strong.label(), "强");
        assert_eq!(Grade::Medium.label(), "中");
        assert_eq!(Grade::Weak.label(), "弱");
        assert_eq!(SectorTier::Tier1.label(), "第一梯队·核心主线");
        assert_eq!(SectorTier::Tier2.label(), "第二梯队·卫星弹性");
        assert_eq!(SectorTier::Watch.label(), "观察区·不入场");
        assert_eq!(SectorTier::Excluded.label(), "排除清单·不关注");
        assert!(grade_sectors(&[]).is_empty());
        assert_eq!(format_tier_list(&[]), "无板块数据");

        let graded = grade_sectors(&[
            board("双强", 20.0, 10e8),
            board("技术强", 15.0, -1e8),
            board("基准甲", 1.0, 1e8),
            board("基准乙", 0.5, 0.5e8),
            board("基准丙", 0.2, 0.2e8),
        ]);
        assert!(graded.iter().any(|sector| sector.tier == SectorTier::Tier1));
        assert!(graded.iter().any(|sector| sector.tier == SectorTier::Tier2));
        let rendered = format_tier_list(&graded);
        assert!(rendered.contains("第一梯队"));
        assert!(rendered.contains("第二梯队"));
        assert!(rendered.contains("双强"));
        assert!(graded.iter().all(|sector| !sector.code.is_empty()));
        assert!(graded.iter().all(|sector| sector.main_inflow.is_finite()));
    }
}
