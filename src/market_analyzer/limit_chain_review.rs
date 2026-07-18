//! v12 MVP4-4.2: 连板高度 + 涨停产业链聚合 (limit_chain_review).
//!
//! 设计: 从 sector_history (按日期+概念存储) 聚合, 输出 ChainLine (R-03 模板入参).
//!       连板高度: 1=首板, 2=二板, 3+=三板以上.
//!       数据缺失时降级标注 (BR-005 §13 准确性).

use serde::{Deserialize, Serialize};

/// 单标的连板统计
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StockLimitStats {
    pub code: String,
    pub name: String,
    pub chain: String,
    pub board_level: u8, // 1=首板, 2=二板, 3+=三板+
    pub is_limit_up_today: bool,
    pub is_first_board: bool,
    pub consecutive_days: u32, // 连续涨停天数
}

/// 产业链聚合行 (R-03 模板入参)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainAggregate {
    pub chain: String,
    pub limit_up_n: u32,
    pub first_n: u32,
    pub consec_n: u32,
    pub heat_stage: String, // HeatUp/MainUp/Range
    pub leader_name: String,
    pub leader_code: String,
    pub leader_boards: u32,
    pub followers: Vec<String>,
    pub watch_point: String,
    pub data_degraded: bool,
}

/// 输入: 涨停扫描结果 (来自 sector_monitor 或 scanner)
#[derive(Clone, Debug, Default)]
pub struct LimitChainInput {
    pub stocks: Vec<StockLimitStats>,
    /// 数据源是否完整 (来自权威接口 vs 推断)
    pub source_complete: bool,
}

/// v12 MVP4-4.2 主聚合: 按 chain 分组 + 选龙头
///
/// 龙头规则:
///   1. board_level 最高
///   2. 同级取连续涨停天数最多
///   3. 同级取 is_first_board=false (非首板, 已被市场验证)
pub fn aggregate(input: &LimitChainInput) -> Vec<ChainAggregate> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<StockLimitStats>> = BTreeMap::new();
    for s in &input.stocks {
        groups.entry(s.chain.clone()).or_default().push(s.clone());
    }

    let mut result = Vec::new();
    for (chain, mut stocks) in groups {
        // 按 board_level 降序, 同级按 consecutive_days 降序, 同级按 is_first_board=false 优先
        stocks.sort_by(|a, b| {
            b.board_level
                .cmp(&a.board_level)
                .then(b.consecutive_days.cmp(&a.consecutive_days))
                .then(a.is_first_board.cmp(&b.is_first_board)) // false < true, 非首板排前
        });

        let leader = stocks.first().cloned().unwrap_or_default();
        let followers: Vec<String> = stocks
            .iter()
            .skip(1)
            .take(5)
            .map(|s| s.name.clone())
            .collect();

        let first_n = stocks.iter().filter(|s| s.is_first_board).count() as u32;
        let consec_n = stocks.iter().filter(|s| s.board_level >= 2).count() as u32;

        let heat_stage = if leader.board_level >= 5 {
            "MainUp"
        } else if leader.board_level >= 3 {
            "HeatUp"
        } else {
            "Range"
        }
        .to_string();

        result.push(ChainAggregate {
            chain: chain.clone(),
            limit_up_n: stocks.len() as u32,
            first_n,
            consec_n,
            heat_stage,
            leader_name: leader.name,
            leader_code: leader.code,
            leader_boards: leader.board_level as u32,
            followers,
            watch_point: String::new(),
            data_degraded: !input.source_complete,
        });
    }
    result
}

/// 工具: 按 code 查 board_level
pub fn get_board_level(stocks: &[StockLimitStats], code: &str) -> Option<u8> {
    stocks
        .iter()
        .find(|s| s.code == code)
        .map(|s| s.board_level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(
        code: &str,
        name: &str,
        chain: &str,
        level: u8,
        consec: u32,
        first: bool,
    ) -> StockLimitStats {
        StockLimitStats {
            code: code.to_string(),
            name: name.to_string(),
            chain: chain.to_string(),
            board_level: level,
            is_limit_up_today: true,
            is_first_board: first,
            consecutive_days: consec,
        }
    }

    #[test]
    fn empty_input_returns_empty() {
        let r = aggregate(&LimitChainInput::default());
        assert!(r.is_empty());
    }

    #[test]
    fn single_chain_aggregate() {
        let input = LimitChainInput {
            stocks: vec![
                s("TEST_CODE_688001", "龙头", "AI算力", 4, 4, false),
                s("TEST_CODE_688002", "次龙", "AI算力", 2, 2, false),
                s("TEST_CODE_688003", "首板", "AI算力", 1, 1, true),
            ],
            source_complete: true,
        };
        let r = aggregate(&input);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].chain, "AI算力");
        assert_eq!(r[0].limit_up_n, 3);
        assert_eq!(r[0].first_n, 1);
        assert_eq!(r[0].consec_n, 2);
        // 龙头: 4 板 (最高)
        assert_eq!(r[0].leader_name, "龙头");
        assert_eq!(r[0].leader_boards, 4);
        assert!(!r[0].data_degraded);
    }

    #[test]
    fn multiple_chains_split() {
        let input = LimitChainInput {
            stocks: vec![
                s("TEST_CODE_000001", "A1", "AI", 3, 3, false),
                s("TEST_CODE_000002", "B1", "机器人", 2, 2, false),
            ],
            source_complete: true,
        };
        let r = aggregate(&input);
        assert_eq!(r.len(), 2);
        let ai = r.iter().find(|x| x.chain == "AI").unwrap();
        let robot = r.iter().find(|x| x.chain == "机器人").unwrap();
        assert_eq!(ai.leader_name, "A1");
        assert_eq!(robot.leader_name, "B1");
    }

    #[test]
    fn leader_selection_higher_board_wins() {
        let input = LimitChainInput {
            stocks: vec![
                s("TEST_CODE_000001", "A", "X", 1, 1, true),
                s("TEST_CODE_000002", "B", "X", 3, 3, false),
            ],
            source_complete: true,
        };
        let r = aggregate(&input);
        assert_eq!(r[0].leader_name, "B", "3 板应胜 1 板");
    }

    #[test]
    fn heat_stage_brackets() {
        // 5 板以上 → MainUp
        let input = LimitChainInput {
            stocks: vec![s("A", "A", "X", 6, 6, false)],
            source_complete: true,
        };
        let r = aggregate(&input);
        assert_eq!(r[0].heat_stage, "MainUp");

        // 3 板 → HeatUp
        let mut inp = input.clone();
        inp.stocks = vec![s("A", "A", "X", 3, 3, false)];
        let r = aggregate(&inp);
        assert_eq!(r[0].heat_stage, "HeatUp");
    }

    #[test]
    fn data_degraded_flag() {
        let input = LimitChainInput {
            stocks: vec![s("A", "A", "X", 3, 3, false)],
            source_complete: false, // 数据源不完整
        };
        let r = aggregate(&input);
        assert!(r[0].data_degraded);
    }

    #[test]
    fn get_board_level_lookup() {
        let stocks = vec![s("A", "A", "X", 3, 3, false)];
        assert_eq!(get_board_level(&stocks, "A"), Some(3));
        assert_eq!(get_board_level(&stocks, "B"), None);
    }

    #[test]
    fn followers_capped_at_5() {
        let mut stocks = Vec::new();
        for i in 0..10 {
            stocks.push(s(
                &format!("C{:03}", i),
                &format!("N{}", i),
                "X",
                1,
                1,
                true,
            ));
        }
        let input = LimitChainInput {
            stocks,
            source_complete: true,
        };
        let r = aggregate(&input);
        assert_eq!(r[0].followers.len(), 5, "followers 上限 5 个");
    }
}
