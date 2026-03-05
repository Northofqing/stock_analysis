// A股自选股智能分析系统 - 库入口

pub mod search_service;
pub mod analyzer;
pub mod trend_analyzer;
pub mod database;
pub mod models;
pub mod schema;
pub mod market_data;
pub mod market_analyzer;
pub mod notification;
pub mod enums;
pub mod data_provider;
pub mod pipeline;
pub mod lhb_analyzer;
pub mod sharpe_calculator;
pub mod chart_generator;
pub mod multi_factor_strategy;
pub mod backtest;

pub use search_service::{
    get_search_service, 
    SearchProvider, 
    SearchResult, 
    SearchResponse, 
    SearchService,
    TavilySearchProvider,
    SerpAPISearchProvider,
    BochaSearchProvider,
};

pub use analyzer::{
    get_analyzer,
    AnalysisResult,
    GeminiAnalyzer,
    GeminiConfig,
};

pub use trend_analyzer::{
    StockTrendAnalyzer,
    TrendAnalysisResult,
    StockData,
    TrendStatus,
    VolumeStatus,
    BuySignal,
    analyze_stock,
};

pub use database::{
    DatabaseManager,
    get_db,
    StockDailyRecord,
    AnalysisContext,
};

pub use models::{
    StockDaily,
    NewStockDaily,
    UpdateStockDaily,
    MaStatus,
    NewAnalysisResult,
    AnalysisResultRecord,
};

pub use lhb_analyzer::{
    LhbDataFetcher,
    LhbRecord,
    LhbSeat,
    LhbAnalysis,
};
