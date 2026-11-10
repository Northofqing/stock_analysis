pub mod multi_agent;
pub mod tool;
pub mod tools;
pub mod tools_chip;
pub mod tools_money_flow;
pub mod tools_news;
pub mod tools_research;
pub mod tools_sector;
// v12 MVP-4 §7.8
pub mod auction_agent;

// Facade re-exports — 外部模块只应通过 `crate::agent` 访问，不直接导入子模块
pub use multi_agent::build_slices;
pub use tool::Tool;
pub use tools::FetchFinancialTool;
pub use tools_chip::FetchChipDistributionTool;
pub use tools_money_flow::FetchFundFlowTool;
pub use tools_news::FetchNewsTool;
pub use tools_research::FetchResearchTool;
pub use tools_sector::FetchSectorTool;
