pub mod state;
pub mod context;
pub mod tool;
pub mod toolbelt;
pub mod loop_runner;
pub mod validation;
pub mod tools;
pub mod tools_sector;
pub mod tools_research;
pub mod tools_money_flow;
pub mod tools_chip;
pub mod tools_news;
pub mod multi_agent;

// Facade re-exports — 外部模块只应通过 `crate::agent` 访问，不直接导入子模块
pub use loop_runner::AgentRunner;
pub use multi_agent::build_slices;
pub use tool::Tool;
pub use toolbelt::Toolbelt;
pub use tools::FetchFinancialTool;
pub use tools_chip::FetchChipDistributionTool;
pub use tools_money_flow::FetchFundFlowTool;
pub use tools_news::FetchNewsTool;
pub use tools_research::FetchResearchTool;
pub use tools_sector::FetchSectorTool;
pub use validation::ValidationEngine;
