//! P4 M2: data_gateway → gRPC 桥 (双进程模式: monitor 连独立 grpc_market_server)。
//! env 契约:
//! - `DATA_GATEWAY_GRPC=1` 启用 (缺省 library 模式, 出声默认 v15.x);
//! - `GRPC_MARKET_ADDR` 服务端地址 (默认 http://127.0.0.1:18082);
//! - `DATA_GATEWAY_GRPC_DISABLED=RealtimeQuotes,HistoricalBars` 按 op 名 opt-out。
//! fail-closed: 服务端不可达 / 证据链缺失 → GatewayError::unavailable
//! (retryable=true) 或 invalid_evidence, 绝不静默回退 library。
//!
//! 同步方法 (realtime_quotes/daily_bars) 在 spawn_blocking 线程里调用 →
//! Handle::block_on; 纯同步线程 → 静态 BRIDGE_RUNTIME。
pub mod convert;

use crate::data_gateway::{
    GatewayBatch, GatewayError, MarketMinutePoint, MarketMoneyFlow, MarketOrderBook,
    MarketSecurityMetadata, RealtimeMarketQuote,
};
use crate::data_provider::KlineData;
use crate::grpc_client::client::GrpcMarketClient;
use crate::grpc_client::envelope::QueryResult;
use crate::grpc_client::pb::magic::market::v1::Operation;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// 桥单例缓存 (l6_sink OnceLock 模式): None = 未初始化 (首次调用时连接)。
static SOURCE: OnceLock<Mutex<Option<Arc<GrpcSource>>>> = OnceLock::new();

/// 纯同步线程 (无 tokio runtime) 的 block_on 载体。
static BRIDGE_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

const DEFAULT_ADDR: &str = "http://127.0.0.1:18082";

/// 网关钩子入口: DATA_GATEWAY_GRPC=1 且 op 未被 DISABLED → Some(Arc<GrpcSource>)
/// (惰性连接, 失败不缓存); 否则 Ok(None) (library 路径)。
/// 连接失败 → Err(unavailable retryable) (fail-closed)。
pub fn bridge_for(op: &str) -> Result<Option<Arc<GrpcSource>>, GatewayError> {
    if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") {
        return Ok(None);
    }
    let disabled = std::env::var("DATA_GATEWAY_GRPC_DISABLED").unwrap_or_default();
    if disabled.split(',').any(|name| name.trim() == op) {
        log::warn!(
            "[data_gateway] gRPC 桥: op {op} 被 DATA_GATEWAY_GRPC_DISABLED 排除, 走 library"
        );
        return Ok(None);
    }
    let cell = SOURCE.get_or_init(|| Mutex::new(None));
    if let Some(source) = cell.lock().unwrap().as_ref() {
        return Ok(Some(source.clone()));
    }
    // 连接是惰性的: 此处只注册桥实例 (连接在首个方法调用 ensure_connected 做,
    // 避免 block_on 在 async 上下文 panic; 失败不缓存语义在方法层保持)。
    let arc = Arc::new(GrpcSource {
        addr: std::env::var("GRPC_MARKET_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string()),
        client: AsyncMutex::new(None),
    });
    *cell.lock().unwrap() = Some(arc.clone());
    Ok(Some(arc))
}

/// 测试/重置用: 清空桥缓存 (重连)。
pub fn reset_bridge() {
    if let Some(cell) = SOURCE.get() {
        *cell.lock().unwrap() = None;
    }
}

/// block_on 判别器: 有 runtime 线程 (spawn_blocking) → Handle::block_on;
/// 纯同步线程 → 静态 BRIDGE_RUNTIME。
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => BRIDGE_RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                    .expect("grpc bridge runtime 创建失败")
            })
            .block_on(fut),
    }
}

/// gRPC 客户端桥: 每 op 一个查询方法, 内部 client.query (§10 重试语义) + convert。
/// 连接是惰性 async (`ensure_connected`, 在方法层做) — 同步方法在 blocking 线程
/// 经 block_on 调用, async 方法在 runtime worker 调用; 首连放在方法层避免
/// bridge_for 里 block_on 在 async 上下文 panic (tokio 禁止)。
pub struct GrpcSource {
    addr: String,
    /// 连接态缓存: None = 尚未连接成功 (失败不缓存, 下次调用重试)。
    /// tokio Mutex: 跨 await 持有 (Send) — delegate JoinSet spawn 要求。
    client: AsyncMutex<Option<GrpcMarketClient>>,
}

impl GrpcSource {
    async fn ensure_connected(&self) -> Result<(), GatewayError> {
        let mut guard = self.client.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let client = GrpcMarketClient::connect(&self.addr).await.map_err(|e| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                true,
                format!("gRPC 服务端 {} 不可达: {e}", self.addr),
            )
        })?;
        log::info!("[data_gateway] gRPC 桥已连接: server={}", self.addr);
        *guard = Some(client);
        Ok(())
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn query_op(&self, op: Operation, params: Value) -> Result<QueryResult, GatewayError> {
        self.ensure_connected().await?;
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().expect("ensure_connected 后必有 client");
        client.query(op, params).await.map_err(|e| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                true,
                format!(
                    "gRPC {:?} 查询失败: {e}",
                    crate::grpc_contract::ops::method_name(op)
                ),
            )
        })
    }

    // ---------- 6 个首批 op (M2) ----------

    pub async fn realtime_quotes_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        let q = self
            .query_op(Operation::RealtimeQuotes, serde_json::json!({ "codes": codes }))
            .await?;
        convert::realtime_quotes(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        block_on(self.realtime_quotes_async(codes))
    }

    pub async fn minute_data_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        let q = self
            .query_op(Operation::MinuteData, serde_json::json!({ "codes": [code] }))
            .await?;
        convert::minute_data(&q)
    }

    pub fn minute_data(&self, code: &str) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        block_on(self.minute_data_async(code))
    }

    pub async fn order_books_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        let q = self
            .query_op(Operation::OrderBooks, serde_json::json!({ "codes": codes }))
            .await?;
        convert::order_books(&q)
    }

    pub fn order_books(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        block_on(self.order_books_async(codes))
    }

    pub async fn money_flows_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        let q = self
            .query_op(Operation::MoneyFlows, serde_json::json!({ "codes": codes }))
            .await?;
        convert::money_flows(&q)
    }

    pub fn money_flows(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        block_on(self.money_flows_async(codes))
    }

    pub async fn security_metadata_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        // 文档 §8 契约: instruments 对象数组 (exchange 由 code 前缀推导)。
        let params = crate::grpc_contract::params::instruments_for(codes);
        let q = self
            .query_op(Operation::SecurityMetadata, params)
            .await?;
        convert::security_metadata(&q)
    }

    pub fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        block_on(self.security_metadata_async(codes))
    }

    pub async fn daily_bars_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        let q = self
            .query_op(
                Operation::HistoricalBars,
                serde_json::json!({ "codes": [code], "days": days }),
            )
            .await?;
        convert::historical_bars(code, &q)
    }

    /// 同步包装。
    pub fn daily_bars(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        block_on(self.daily_bars_async(code, days))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_disabled_without_env() {
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(bridge_for("RealtimeQuotes").unwrap().is_none());
    }

    #[test]
    fn bridge_disabled_by_op_name() {
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "RealtimeQuotes");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(
            bridge_for("RealtimeQuotes").unwrap().is_none(),
            "DISABLED 命中 → library"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
    }

    #[test]
    fn bridge_enabled_but_unreachable_is_fail_closed() {
        // 连接是惰性的: bridge_for 只注册实例, fail-closed 在方法层
        // (首个查询 ensure_connected 失败 → unavailable retryable)。
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        reset_bridge();
        let bridge = bridge_for("RealtimeQuotes").unwrap().expect("bridge 实例存在");
        let err = bridge.realtime_quotes(&["600519".to_string()]).unwrap_err();
        assert!(err.retryable(), "服务端不可达必须 retryable");
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        reset_bridge();
    }
}
