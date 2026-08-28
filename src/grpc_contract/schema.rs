//! canonical JSON schema 注册表 (合同 §5: 每方法 schema 名/版本冻结;
//! 调用方遇到未知 schema/version 必须停止解析, 不能忽略或猜字段)。
//!
//! 初始以 data_gateway 返回类型的 JSON 为准, 冻结 24 个生产 op,
//! 后续扩展至 40 个（含本地扩展 op，见 ops.rs implemented_operations）；
//! schema 名约定: "<域>.<数据族>", 版本从 1 起。
use crate::grpc_client::pb::magic::market::v1::Operation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSchema {
    pub operation: Operation,
    pub schema_name: &'static str,
    pub schema_version: u32,
}

const SCHEMAS: &[OpSchema] = &[
    OpSchema {
        operation: Operation::RealtimeQuotes,
        schema_name: "market.realtime_quotes",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::HistoricalBars,
        schema_name: "market.historical_bars",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::MinuteData,
        schema_name: "market.minute_data",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::OrderBooks,
        schema_name: "market.order_books",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::MoneyFlows,
        schema_name: "market.money_flows",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::SecurityMetadata,
        schema_name: "market.security_metadata",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::Announcements,
        schema_name: "news.announcements",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::GlobalNews,
        schema_name: "news.global_news",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::EconomicCalendar,
        schema_name: "market.economic_calendar",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::FuturesDelivery,
        schema_name: "market.futures_delivery",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::GlobalIndices,
        schema_name: "market.global_indices",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::BoardDirectory,
        schema_name: "board.directory",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::BoardConstituents,
        schema_name: "board.constituents",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::BoardFlows,
        schema_name: "board.flows",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::LimitPools,
        schema_name: "market.limit_pools",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::StrongStockReasons,
        schema_name: "market.strong_stock_reasons",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::DragonTiger,
        schema_name: "market.dragon_tiger",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::MarketDragonTiger,
        schema_name: "market.market_dragon_tiger",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::MarketRankings,
        schema_name: "market.market_rankings",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::ConceptHits,
        schema_name: "market.concept_hits",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::Consensus,
        schema_name: "market.consensus",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::ResearchReports,
        schema_name: "research.reports",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::BlockTrades,
        schema_name: "market.block_trades",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::NorthboundDaily,
        schema_name: "market.northbound_daily",
        schema_version: 1,
    },
    // M1 扩展 (P4): 8 个 proto 已有 op。
    OpSchema {
        operation: Operation::ForeignExchange,
        schema_name: "market.foreign_exchange",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::FinancialStatements,
        schema_name: "market.financial_statements",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::MarketStatistics,
        schema_name: "market.market_statistics",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::TechnicalBars,
        schema_name: "market.technical_bars",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::CorporateActions,
        schema_name: "market.corporate_actions",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::SemanticSearch,
        schema_name: "market.semantic_search",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::FundFlowSeries,
        schema_name: "market.fund_flow_series",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::ProviderTopNRankings,
        schema_name: "market.provider_top_n_rankings",
        schema_version: 1,
    },
    // M1 扩展 (P4): 6 个新 op (proto 编号 55-60)。
    OpSchema {
        operation: Operation::IndexQuotes,
        schema_name: "market.index_quotes",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::InstrumentNews,
        schema_name: "news.instrument_news",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::IntradayShape,
        schema_name: "market.intraday_shape",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::T0Evidence,
        schema_name: "market.t0_evidence",
        schema_version: 2,
    },
    OpSchema {
        operation: Operation::OutcomeDailyBars,
        schema_name: "market.outcome_daily_bars",
        schema_version: 1,
    },
    OpSchema {
        operation: Operation::UpperLimitPoolReview,
        schema_name: "market.upper_limit_pool_review",
        schema_version: 1,
    },
    // M4c 扩展: A-10 完整 batch (本地扩展 61, monitor 复盘消费)。
    OpSchema {
        operation: Operation::ChainBatch,
        schema_name: "market.chain_batch",
        schema_version: 1,
    },
    // BR-251: 指数专用历史基准批次；wire 绑定请求、批次证据与真实 BR-159 receipt。
    OpSchema {
        operation: Operation::BenchmarkBars,
        schema_name: "market.benchmark_bars",
        schema_version: 1,
    },
];

pub fn schema_for(op: Operation) -> Option<&'static OpSchema> {
    SCHEMAS.iter().find(|s| s.operation == op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_implemented_op_has_frozen_schema() {
        // 40 个已实现 op 全部有 schema (M1 扩展 + M4c ChainBatch + BR-251 BenchmarkBars)。
        assert_eq!(SCHEMAS.len(), 40);
        for op in crate::grpc_contract::ops::implemented_operations() {
            assert!(schema_for(op).is_some(), "op {op:?} 缺 schema");
        }
    }

    #[test]
    fn benchmark_bars_uses_frozen_v1_schema() {
        let schema = schema_for(Operation::BenchmarkBars).expect("BenchmarkBars schema");
        assert_eq!(schema.schema_name, "market.benchmark_bars");
        assert_eq!(schema.schema_version, 1);
    }

    #[test]
    fn t0_evidence_uses_frozen_v2_schema() {
        let schema = schema_for(Operation::T0Evidence).expect("T0Evidence schema");
        assert_eq!(schema.schema_name, "market.t0_evidence");
        assert_eq!(schema.schema_version, 2);
    }

    #[test]
    fn schema_names_are_unique() {
        let mut names: Vec<&str> = SCHEMAS.iter().map(|s| s.schema_name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SCHEMAS.len());
    }
}
