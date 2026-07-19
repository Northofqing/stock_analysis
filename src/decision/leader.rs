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
pub fn identify_leaders(board: &ConceptBoard, components: &[BoardStock]) -> Vec<LeaderRank> {
    if components.is_empty() {
        return vec![];
    }

    let mut sorted: Vec<&BoardStock> = components.iter().collect();
    sorted.sort_by(|a, b| {
        b.amount
            .partial_cmp(&a.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, s)| {
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
        })
        .collect()
}

pub async fn enrich_bom(leaders: &mut [LeaderRank], sector_name: &str) {
    if leaders.is_empty() {
        return;
    }
    let analyzer = crate::analyzer::GeminiAnalyzer::from_env();
    if !analyzer.is_available() {
        return;
    }
    let prompt = build_bom_prompt(leaders, sector_name);

    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        analyzer.call_api_mode(
            &prompt,
            "你是产业链分析师。只输出代码|环节|理由。",
            crate::analyzer::AgentMode::Quick,
        ),
    )
    .await
    {
        Ok(Ok(text)) => apply_bom_response(leaders, &text),
        Ok(Err(e)) => {
            // 修复 P2.3: 之前 silent (let _ = ..), 现在显式 warn
            log::warn!(
                "[P2.3] enrich_bom AI 调用失败: {} (sector={}, leaders={})",
                e,
                sector_name,
                leaders.len()
            );
        }
        Err(_timeout) => {
            log::warn!(
                "[P2.3] enrich_bom AI 调用超时 (5s, sector={}, leaders={})",
                sector_name,
                leaders.len()
            );
        }
    }
}

fn build_bom_prompt(leaders: &[LeaderRank], sector_name: &str) -> String {
    let codes: Vec<String> = leaders
        .iter()
        .map(|leader| format!("{}({})", leader.name, leader.code))
        .collect();
    format!(
        "你是A股产业链分析师。请为以下{}板块的龙头标的标注其在产业链BOM中的核心环节。\n\
         板块：{}\n龙头：{}\n\
         输出格式（每行）：代码|BOM环节|一句话理由\n\
         只输出每行，不要额外解释。",
        sector_name,
        sector_name,
        codes.join("、"),
    )
}

fn apply_bom_response(leaders: &mut [LeaderRank], text: &str) {
    for line in text.lines().take(leaders.len()) {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 2 {
            let code = parts[0].trim();
            let bom = parts[1].trim();
            let reason = parts.get(2).map(|part| part.trim()).unwrap_or("");
            if let Some(leader) = leaders
                .iter_mut()
                .find(|leader| leader.code == code || line.contains(&leader.code))
            {
                leader.bom_position = Some(bom.to_string());
                if !reason.is_empty() {
                    leader.reason = reason.to_string();
                }
            }
        }
    }
}

/// 格式化龙头榜单
pub fn format_leader_list(leaders: &[LeaderRank], sector_name: &str) -> String {
    if leaders.is_empty() {
        return String::new();
    }
    let mut lines = vec![format!("🏆 {}·龙头识别", sector_name)];
    for l in leaders {
        let bom = l.bom_position.as_deref().unwrap_or("——");
        lines.push(format!(
            "  {}. {}({}) BOM:{} — {}",
            l.rank, l.name, l.code, bom, l.reason
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(code: &str, name: &str, amount: f64) -> BoardStock {
        BoardStock {
            code: code.to_string(),
            name: name.to_string(),
            price: 10.0,
            change_pct: 0.0,
            amount,
            vol_ratio: 0.0,
            turnover: 0.0,
        }
    }

    #[test]
    fn test_identify_top3() {
        let board = ConceptBoard {
            code: "BK0001".into(),
            name: "AI算力".into(),
            change_pct: 5.0,
            main_inflow: 1e9,
            leader_name: String::new(),
            vol_ratio: 0.0,
            turnover: 0.0,
            main_net_pct_today: 0.0,
            main_net_pct_5d: 0.0,
        };
        let components = vec![
            stock("TEST_CODE_000001", "龙头A", 50e8),
            stock("TEST_CODE_000002", "龙头B", 30e8),
            stock("TEST_CODE_000003", "龙头C", 20e8),
            stock("TEST_CODE_000004", "杂毛D", 1e8),
        ];
        let leaders = identify_leaders(&board, &components);
        assert_eq!(leaders.len(), 3);
        assert_eq!(leaders[0].code, "TEST_CODE_000001");
        assert_eq!(leaders[0].rank, 1);
        assert_eq!(leaders[2].code, "TEST_CODE_000003");
    }

    #[tokio::test]
    async fn empty_and_formatted_leader_paths_preserve_missing_bom() {
        let board = ConceptBoard {
            code: "BK0001".into(),
            name: "测试板块".into(),
            change_pct: 1.0,
            main_inflow: 1.0,
            leader_name: String::new(),
            vol_ratio: 1.0,
            turnover: 1.0,
            main_net_pct_today: 1.0,
            main_net_pct_5d: 1.0,
        };
        assert!(identify_leaders(&board, &[]).is_empty());
        assert!(format_leader_list(&[], "测试板块").is_empty());
        let mut leaders = vec![
            LeaderRank {
                code: "TEST_CODE_000001".to_string(),
                name: "甲".to_string(),
                rank: 1,
                bom_position: None,
                reason: "成交额第一".to_string(),
            },
            LeaderRank {
                code: "TEST_CODE_000002".to_string(),
                name: "乙".to_string(),
                rank: 2,
                bom_position: Some("上游设备".to_string()),
                reason: "成交额第二".to_string(),
            },
        ];
        let rendered = format_leader_list(&leaders, "测试板块");
        assert!(rendered.contains("BOM:——"));
        assert!(rendered.contains("BOM:上游设备"));
        leaders.clear();
        enrich_bom(&mut leaders, "测试板块").await;
        assert!(leaders.is_empty());
    }

    #[test]
    fn bom_prompt_and_response_apply_only_matching_protocol_rows() {
        let mut leaders = vec![
            LeaderRank {
                code: "TEST_CODE_000001".to_string(),
                name: "测试龙头甲".to_string(),
                rank: 1,
                bom_position: None,
                reason: "原原因甲".to_string(),
            },
            LeaderRank {
                code: "TEST_CODE_000002".to_string(),
                name: "测试龙头乙".to_string(),
                rank: 2,
                bom_position: None,
                reason: "原原因乙".to_string(),
            },
        ];
        let prompt = build_bom_prompt(&leaders, "测试产业链");
        assert!(prompt.contains("测试产业链"));
        assert!(prompt.contains("测试龙头甲(TEST_CODE_000001)"));
        assert!(prompt.contains("测试龙头乙(TEST_CODE_000002)"));

        apply_bom_response(
            &mut leaders,
            "TEST_CODE_000001|上游设备|订单证据\n前缀TEST_CODE_000002后缀|中游制造|",
        );
        assert_eq!(leaders[0].bom_position.as_deref(), Some("上游设备"));
        assert_eq!(leaders[0].reason, "订单证据");
        assert_eq!(leaders[1].bom_position.as_deref(), Some("中游制造"));
        assert_eq!(leaders[1].reason, "原原因乙");

        let before = leaders.clone();
        apply_bom_response(&mut leaders, "坏协议行\nTEST_CODE_999999|未知环节|无关");
        assert_eq!(leaders[0].bom_position, before[0].bom_position);
        assert_eq!(leaders[1].bom_position, before[1].bom_position);
    }
}
