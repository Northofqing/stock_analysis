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
        Ok(handle) => {
            // 修复 v9.4.15 (2026-06-29 production panic):
            // 在 tokio runtime 内直接 handle.block_on(fut) 会 panic
            // "Cannot start a runtime from within a runtime" (占住 worker).
            //
            // 正确做法: 检测 runtime flavor 分支处理:
            //  - multi_thread: block_in_place + block_on 安全 (让出 worker)
            //  - current_thread: block_in_place 会 panic! 这种情况应 panic
            //    带 actionable error (用户应该用 spawn_blocking 隔离 sync API)
            match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(fut))
                }
                _ => {
                    // CurrentThread (旧版 tokio) 或其他 flavor:
                    // block_in_place 会 panic! 用 actionable error.
                    // 修复 v9.4.20 (2026-06-29 codex M-2): 加 [BLOCK_ON_ASYNC_FLAVOR_ERROR] tag
                    // + 英文 lead-line, 让 log scraper grep 友好.
                    panic!(
                        "[BLOCK_ON_ASYNC_FLAVOR_ERROR] cannot block_on in {:?} runtime\n\
                         block_on_async: cannot call handle.block_on() inside a current_thread runtime\n\n\
                         Fix:\n  \
                         1) Call from tokio::task::spawn_blocking (not from async fn body directly)\n  \
                         2) Change #[tokio::main] to #[tokio::main(flavor = \"multi_thread\")]\n  \
                         3) Build your own multi_thread runtime: Builder::new_multi_thread().enable_all().build()\n\n\
                         Ref: v9.4.15 (2026-06-29) audit Top10#5 修复",
                        handle.runtime_flavor()
                    );
                }
            }
        }
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
///
/// P0-5 修订: 之前直接 `handle.block_on(work)` 不检查 runtime_flavor,
/// 在 current_thread runtime 里调用会 panic "Cannot start a runtime from within a runtime".
/// (实际由 monitor --review 的 MultiAgent 路径触发, 2026-07-03)
/// 修复: 跟 `block_on_async` 对齐 — 按 runtime_flavor 分支处理.
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
        Ok(handle) => {
            // 跟 block_on_async 对齐: 按 runtime_flavor 分支处理
            match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(work))
                }
                _ => {
                    // CurrentThread (旧版 tokio) 或其他 flavor:
                    // block_in_place 会 panic! 用 actionable error (跟 block_on_async 一致).
                    panic!(
                        "[BLOCK_ON_ASYNC_FLAVOR_ERROR] cannot block_on in {:?} runtime\n\
                         block_on_async_with_timeout: cannot call handle.block_on() inside a current_thread runtime\n\n\
                         Fix:\n  \
                         1) Call from tokio::task::spawn_blocking (not from async fn body directly)\n  \
                         2) Change #[tokio::main] to #[tokio::main(flavor = \"multi_thread\")]\n  \
                         3) Build your own multi_thread runtime: Builder::new_multi_thread().enable_all().build()\n\n\
                         Ref: P0-5 (2026-07-03) monitor --review MultiAgent panic",
                        handle.runtime_flavor()
                    );
                }
            }
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("block_on_async_with_timeout: 创建临时 runtime 失败");
            rt.block_on(work)
        }
    }
}

// 修复 Top10#7 (2026-06-29 audit): 共享 HTTP client (4 个预配置, 避免 28 处散落 builder)
pub mod http_client;

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    /// P0-5: `block_on_async_with_timeout` 在 current_thread runtime 里 panic 时,
    /// 错误信息含 actionable tag `[BLOCK_ON_ASYNC_FLAVOR_ERROR]` (跟 block_on_async 对齐).
    ///
    /// 之前直接 `handle.block_on(work)` 不检查 runtime_flavor, 报 "Cannot start a runtime
    /// from within a runtime" 让用户困惑. 现在明确告诉用户:
    ///   1) Call from spawn_blocking
    ///   2) Change #[tokio::main] to multi_thread flavor
    ///   3) Build your own multi_thread runtime
    #[test]
    fn block_on_async_with_timeout_panics_with_flavor_error_in_current_thread() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let unwind = panic::catch_unwind(|| {
            rt.block_on(async {
                block_on_async_with_timeout(async {}, 1).unwrap();
            });
        });
        let panic_payload = unwind.err().expect("current_thread runtime 内调应 panic");
        let panic_msg = extract_panic_message(&panic_payload);
        assert!(
            panic_msg.contains("BLOCK_ON_ASYNC_FLAVOR_ERROR"),
            "panic message 应含 actionable tag, 实际: {}",
            panic_msg
        );
        assert!(
            panic_msg.contains("P0-5"),
            "panic message 应含 P0-5 标记便于追踪, 实际: {}",
            panic_msg
        );
    }

    /// P0-5: `block_on_async_with_timeout` 在 multi_thread runtime 里正常工作.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn block_on_async_with_timeout_works_in_multi_thread() {
        let result = block_on_async_with_timeout(
            async { 42_u32 + 1 },
            5,
        );
        assert_eq!(result.unwrap(), 43);
    }

    /// helper: 把 `Box<dyn Any>` (panic payload) 转成可读字符串.
    fn extract_panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<&'static str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        }
    }
}
