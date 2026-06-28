//! 龙头识别 — Top 3 龙头 + AI BOM 拆解。

use crate::market_analyzer::sector_monitor::{BoardStock, ConceptBoard};

#[derive(Debug, Clone)]
pub struct LeaderRank {
    pub code: String,
    pub name: String,
    pub rank: u8,
    pub bom_position: Option<String>,
    pub reason: String,
}

/// 从板块成份股中识别 Top 3 龙头（按成交额排序）
pub fn identify_leaders(
    board: &ConceptBoard,
    components: &[BoardStock],
) -> Vec<LeaderRank> {
    if components.is_empty() { return vec![]; }

    let mut sorted: Vec<&BoardStock> = components.iter().collect();
    sorted.sort_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap_or(std::cmp::Ordering::Equal));

    sorted.iter().take(3).enumerate().map(|(i, s)| {
        LeaderRank {
            code: s.code.clone(),
            name: s.name.clone(),
            rank: (i + 1) as u8,
            bom_position: None, // AI 异步补充
            reason: if i == 0 {
                format!("{}板块成交额第一", board.name)
            } else {
                format!("{}板块成交额第{}", board.name, i + 1)
            },
        }
    }).collect()
}

/// 用 AI 补充 BOM 环节信息（同一板块同一天缓存，AI 不可用时静默降级）
use log::warn;

pub async fn enrich_bom(leaders: &mut [LeaderRank], sector_name: &str) {
    let analyzer = crate::analyzer::GeminiAnalyzer::from_env();
    if !analyzer.is_available() { return; }
    if leaders.is_empty() { return; }

    let codes: Vec<String> = leaders.iter().map(|l| format!("{}({})", l.name, l.code)).collect();
    let prompt = format!(
        "你是A股产业链分析师。请为以下{}板块的龙头标的标注其在产业链BOM中的核心环节。\n\
         板块：{}\n龙头：{}\n\
         输出格式（每行）：代码|BOM环节|一句话理由\n\
         只输出每行，不要额外解释。",
        sector_name, sector_name, codes.join("、"),
    );

    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        analyzer.call_api_mode(&prompt, "你是产业链分析师。只输出代码|环节|理由。", crate::analyzer::AgentMode::Quick),
    ).await {
        Ok(Ok(text)) => {
            for line in text.lines().take(leaders.len()) {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 2 {
                    let code = parts[0].trim();
                    let bom = parts[1].trim();
                    let reason = parts.get(2).map(|s| s.trim()).unwrap_or("");
                    if let Some(leader) = leaders.iter_mut().find(|l| l.code == code || line.contains(&l.code)) {
                        leader.bom_position = Some(bom.to_string());
                        if !reason.is_empty() { leader.reason = reason.to_string(); }
                    }
                }
            }
        }
        Ok(Err(e)) => {
            // 修复 P2.3: 之前 silent (let _ = ..), 现在显式 warn
            log::warn!("[P2.3] enrich_bom AI 调用失败: {} (sector={}, leaders={})", e, sector_name, leaders.len());
        }
        Err(_timeout) => {
            log::warn!("[P2.3] enrich_bom AI 调用超时 (5s, sector={}, leaders={})", sector_name, leaders.len());
        }
    }
}

/// 格式化龙头榜单
pub fn format_leader_list(leaders: &[LeaderRank], sector_name: &str) -> String {
    if leaders.is_empty() { return String::new(); }
    let mut lines = vec![format!("🏆 {}·龙头识别", sector_name)];
    for l in leaders {
        let bom = l.bom_position.as_deref().unwrap_or("——");
        lines.push(format!("  {}. {}({}) BOM:{} — {}", l.rank, l.name, l.code, bom, l.reason));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(code: &str, name: &str, amount: f64) -> BoardStock {
        BoardStock {
            code: code.to_string(), name: name.to_string(),
            change_pct: 0.0, amount, vol_ratio: 0.0, turnover: 0.0,
        }
    }

    #[test]
    fn test_identify_top3() {
        let board = ConceptBoard {
            code: "BK0001".into(), name: "AI算力".into(),
            change_pct: 5.0, main_inflow: 1e9,
            leader_name: String::new(), vol_ratio: 0.0, turnover: 0.0,
            main_net_pct_today: 0.0, main_net_pct_5d: 0.0,
        };
        let components = vec![
            stock("000001", "龙头A", 50e8),
            stock("000002", "龙头B", 30e8),
            stock("000003", "龙头C", 20e8),
            stock("000004", "杂毛D", 1e8),
        ];
        let leaders = identify_leaders(&board, &components);
        assert_eq!(leaders.len(), 3);
        assert_eq!(leaders[0].code, "000001");
        assert_eq!(leaders[0].rank, 1);
        assert_eq!(leaders[2].code, "000003");
    }
}
