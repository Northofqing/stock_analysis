// A股自选股智能分析系统 - 库入口

pub mod errors;
pub mod config;
pub mod breakout;
pub mod portfolio;
pub mod review;
pub mod signal;
pub mod opportunity;
pub mod decision;
pub mod risk;
pub mod traits;
pub mod strategy;
pub mod search_service;
pub mod analyzer;
pub mod trend_analyzer;
pub mod database;
pub mod models;
pub mod types;
pub mod schema;
pub mod calendar;
pub mod market_data;
pub mod market_analyzer;
pub mod notification;
pub mod enums;
pub mod data_provider;
pub mod indicators;
pub mod pipeline;
pub mod lhb_analyzer;
pub mod monitor;
pub mod sharpe_calculator;
pub mod chart_generator;

// 向外兼容：旧模块路径指向新的 strategy 子模块
pub use strategy::core as backtest;
pub use strategy::bollinger_zscore as bollinger_zscore_strategy;
pub use strategy::multi_factor as multi_factor_strategy;

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
pub mod agent;
pub mod deep_analyzer;
pub mod trading;

// ========================================================================
// 修复 Top10#5 (2026-06-29 audit): block_on 统一桥接
// ========================================================================
//
// 背景: 全代码库有 25+ 处 `block_on` / `Runtime::new` / `Handle::try_current`,
// 在不同文件用 4 种不同 pattern:
//   1. `let rt = tokio::runtime::Runtime::new()?; rt.block_on(fut)` (RsiOptimize)
//   2. `match Handle::try_current() { Ok(h) => h.block_on(fut), Err(_) => rt.block_on(fut) }`
//      (gtimg_provider 4 处)
//   3. `let handle = Handle::try_current().ok()?; handle.block_on(fut)` (8 个 data_provider 文件)
//   4. `Handle::current().block_on(fut)` (statistics.rs, eastmoney_provider.rs)
//
// 问题:
//   - 每处手写 pattern,容易出错 (e.g. gtimg_provider 第 100 行错误处理)
//   - 多 Runtime 嵌套 = 40 个空闲 worker
//   - `block_on` 不带超时,可能永久阻塞 worker
//
// 修法: 统一 `block_on_async` 函数 (本文件底部),所有调用方改成调它.
// 调用方 import: `use stock_analysis::block_on_async;`
// 行为:
//   - 在 tokio runtime 内: `Handle::current().block_on(fut)` (不创建新 runtime)
//   - 不在 runtime 内: 创建 current_thread runtime 临时跑
//   - 加超时 (默认 30s) 防永久阻塞

/// 修复 Top10#5: 统一 block_on 桥接, 替代 25+ 处散落 pattern.
/// 用法: `block_on_async(async { fetch_async().await })`
pub fn block_on_async<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    use tokio::runtime::Handle;
    match Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => {
            // 不在 tokio runtime 内 (e.g. 同步 binary, 测试).
            // 建 current_thread runtime 临时跑, 避免多 Runtime 嵌套.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("block_on_async: 创建临时 current_thread runtime 失败");
            rt.block_on(fut)
        }
    }
}

/// 修复 Top10#5: 统一 block_on + 超时, 防永久阻塞.
/// 默认 30s 超时足够大多数 data_provider HTTP 调用, 长任务 (e.g. backtest) 显式传大值.
pub fn block_on_async_with_timeout<F, T>(fut: F, timeout_secs: u64) -> Result<T, String>
where
    F: std::future::Future<Output = T>,
{
    use tokio::runtime::Handle;
    let work = async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            fut,
        )
        .await
        {
            Ok(v) => Ok(v),
            Err(_) => Err(format!(
                "block_on_async_with_timeout: 任务超过 {timeout_secs}s 超时"
            )),
        }
    };
    match Handle::try_current() {
        Ok(handle) => handle.block_on(work),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("block_on_async_with_timeout: 创建临时 runtime 失败");
            rt.block_on(work)
        }
    }
}
