# gRPC 数据通道实施计划（Spec P0-P2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 gRPC 数据通道——用 `grpc/market.proto` 生成客户端与服务端骨架，客户端 `GrpcMarketClient` 可查询生产实际用到的全部数据族（24 op），服务端 `grpc_market_server` 委托 `data_gateway` 取真实数据并生成 TDX 异动事件流。

**Architecture:** 服务端逻辑放库模块 `src/grpc_server/`（可被集成测试直接构造），`src/bin/grpc_market_server.rs` 是薄入口；客户端在 `src/grpc_client/`；schema 注册表在 `src/grpc_contract/`（双端共享）。测试用 fixture 模式（`fixture_mode: true`）离线确定性；真实 provider 路径由手工运行 server 二进制验证。

**Tech Stack:** tonic 0.14（client+server 生成）、prost 0.14、tonic-build 0.14（build-dependency）、tokio-stream 0.1（ReceiverStream）、tokio、serde_json、anyhow。

**Spec:** `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md`（已批准，commit 61ee717）

## Global Constraints

- proto 唯一合同源 `grpc/market.proto`：**不修改、不复制**。生成代码进 OUT_DIR，不提交。
- **现有程序零改动**（P0-P2 只加文件）：monitor 二进制、14 处 `push_governor_v3`、现有 289 单测不动；`Cargo.toml` 只加 tonic/prost/tokio-stream/tonic-build。
- 判断回归用**模块级测试**（全量 `cargo test --lib` 预存 23-28 个顺序依赖 flaky，pre-existing）。
- v15.x 规则：默认值必须是"出声"状态；env var 显式声明才能启用静默默认值。
- 服务端默认监听 `127.0.0.1:18082`（避开 MagicLaw 18011）。
- 合同语义（§6）：`admission=ADMITTED` 才能当生产数据；`complete=false` 不当成功批次；`source_at` 缺失不填充；未知 schema/version 必须拒绝，不猜字段。
- 认证：token 只进 gRPC metadata（`GRPC_MARKET_TOKEN`），不进请求体/URL/日志。
- commit 沿用仓库 conventional commits 惯例；BR 编号由用户分配后补充。
- 测试字符串不得进生产 push 路径（本计划不涉及 push，P3 计划再启用该约束）。

---

### Task 1: 依赖 + build.rs + 生成代码接入

**Files:**
- Modify: `Cargo.toml`（[dependencies] 加 tonic/tonic-prost/prost/tokio-stream；[build-dependencies] 加 tonic-prost-build）
- Create: `build.rs`
- Create: `src/grpc_client/mod.rs`、`src/grpc_client/pb.rs`、`src/grpc_client/auth.rs`、`src/grpc_client/client.rs`、`src/grpc_client/envelope.rs`、`src/grpc_client/errors.rs`、`src/grpc_client/retry.rs`（后 5 个为空骨架，Task 4-7 填充）
- Create: `src/grpc_contract/mod.rs`、`src/grpc_contract/ops.rs`、`src/grpc_contract/schema.rs`、`src/grpc_contract/validate.rs`（后 3 个为空骨架，Task 2/3 填充）
- Modify: `src/lib.rs`（加 `pub mod grpc_contract; pub mod grpc_client;`）

**Interfaces:**
- Consumes: 无（第一个任务）
- Produces: 生成代码模块 `crate::grpc_client::pb::magic::market::v1`，含 `QueryRequest`/`QueryResponse`/`CanonicalPayload`/`RequestContext`/`Operation`/`AdmissionState`/`MarketEventEnvelope`/`EventCursor`/`EventFilter`/`SubscribeRequest`/`ReplayRequest`/`ListenerStatusRequest`/`ListenerStatusResponse`/`HealthRequest`/`HealthResponse`/`CapabilitiesRequest`/`CapabilitiesResponse`/`Capability`/`ErrorDetail`；服务 trait `system_service_server::SystemService`、`market_data_service_server::MarketDataService`、`market_event_service_server::MarketEventService`；客户端 `system_service_client::SystemServiceClient`、`market_data_service_client::MarketDataServiceClient`、`market_event_service_client::MarketEventServiceClient`。

- [x] **Step 1: 加依赖**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
tonic = "0.14"
prost = "0.14"
tokio-stream = "0.1"
```

`[build-dependencies]` 追加（文件末尾）：

```toml
[build-dependencies]
tonic-build = "0.14"
```

- [x] **Step 2: 写 build.rs**

```rust
fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["grpc/market.proto"], &["grpc/"])
        .expect("compile grpc/market.proto (合同唯一源, 不得修改)");
    println!("cargo:rerun-if-changed=grpc/market.proto");
}
```

- [x] **Step 3: 建模块**

`src/grpc_client/pb.rs`：

```rust
//! tonic-prost-build 生成的 magic.market.v1 代码 (OUT_DIR, 不提交)。
//! prost-build 0.14 无条件生成包嵌套模块 (generate-modules 已是默认),
//! 生成文件内自带 `pub mod magic { pub mod market { pub mod v1 { ... } } }`,
//! 所以这里**不能再**手写嵌套, 否则双重嵌套。用法:
//! `use crate::grpc_client::pb::magic::market::v1::QueryRequest;`
pub mod pb {
    tonic::include_proto!("magic.market.v1");
}
```

`src/grpc_client/mod.rs`：

```rust
//! gRPC 客户端网络层 (合同: grpc/grpc-external-api.md)。
pub mod auth;
pub mod client;
pub mod envelope;
pub mod errors;
pub mod pb;
pub mod retry;
```

（先建文件骨架；后续 Task 逐个填充 `auth`/`client`/`envelope`/`errors`/`retry`。为通过编译，Task 1 把未实现模块写成空 `//! 待 Task N 填充` 注释模块即可，Task 4-7 逐个替换。）

`src/grpc_contract/mod.rs`（空骨架，Task 2/3 填充）：

```rust
//! gRPC 合同注册表 (schema 名/版本/校验, 服务端与客户端共享)。
//! Task 2 加 `pub mod ops;`; Task 3 加 `pub mod schema; pub mod validate;`
//! (先建对应空文件, 否则 `pub mod` 声明导致编译失败)。
```

同时创建 5 个空骨架文件（各只含一行文档注释，Task 4-7 逐个替换为完整实现）：

```rust
//! 待 Task 4 填充: request_id + QueryRequest/QueryResponse 信封。
```
（`src/grpc_client/envelope.rs` 同上格式；`auth.rs`/`client.rs`/`errors.rs`/`retry.rs` 分别标注 Task 6/7/5/5。）

以及 `src/grpc_contract/ops.rs`/`schema.rs`/`validate.rs` 三个空骨架（各含 `//! 待 Task 2/3 填充` 文档注释）。

`src/lib.rs` 追加：

```rust
pub mod grpc_contract;
pub mod grpc_client;
```

- [x] **Step 4: 编译验证**

Run: `cargo build --lib 2>&1 | tail -5`
Expected: exit 0。若 tonic/prost 版本解析失败，改用 `cargo add tonic@0.14 prost@0.14 tonic-build@0.14 --build` 让 cargo 解析，保持 0.14 大版本。

- [x] **Step 5: 生成代码冒烟测试**

`src/grpc_client/pb.rs` 末尾加单测：

```rust
#[cfg(test)]
mod tests {
    use super::pb::magic::market::v1::{AdmissionState, CanonicalPayload, QueryResponse};

    #[test]
    fn generated_types_roundtrip() {
        let payload = CanonicalPayload {
            schema: "market.realtime_quotes".to_string(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".to_string(),
            data: b"[]".to_vec(),
        };
        let resp = QueryResponse {
            request_id: "r-1".to_string(),
            operation: 3, // OPERATION_REALTIME_QUOTES
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "2026-08-13T10:00:00+08:00".to_string(),
            source_at: "2026-08-13T10:00:00+08:00".to_string(),
            records: vec![payload],
        };
        let bytes = prost::Message::encode_to_vec(&resp);
        let decoded = QueryResponse::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.request_id, "r-1");
        assert_eq!(decoded.records.len(), 1);
        assert_eq!(decoded.records[0].schema, "market.realtime_quotes");
    }
}
```

- [x] **Step 6: 跑测试**

Run: `cargo test --lib grpc_client::pb:: 2>&1 | tail -5`
Expected: PASS（1 passed）。同时 `cargo build --lib` exit 0。

- [x] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock build.rs src/lib.rs src/grpc_client/
git commit -m "feat(grpc): P0 tonic 依赖 + market.proto 生成代码接入 (pb 模块 + 冒烟测试)"
```

**实施偏差记录（0.14 实测，2026-08-15）** — 计划草拟时基于 0.14 前 API 假设，实施时实测修正：

1. **`tonic_build::configure()` 在 0.14 不存在** — tonic 0.14 破坏性重构把 prost 集成拆到 `tonic-prost-build`（"Prost functionality has been moved to tonic-prost-build" — tonic-build-0.14.6 lib.rs 注释）。build.rs 用 `tonic_prost_build::configure().compile_protos(...)`（API 等价）。Cargo.toml: [build-dependencies] 只留 `tonic-prost-build = "0.14"`（传递依赖 tonic-build）。
2. **额外运行时依赖 `tonic-prost = "0.14"`** — tonic-prost-build 生成代码默认 codec_path = `tonic_prost::ProstCodec`，缺它会报 120×E0433。
3. **生成文件是扁平结构，无 `mod magic` 嵌套** — tonic-prost-build 0.14 实测生成 magic.market.v1.rs 顶层直接是 message struct + service 模块（计划注释假设的 generate-modules 嵌套默认行为不成立）。`src/grpc_client/pb.rs` 手写 `pub mod magic { pub mod market { pub mod v1 { tonic::include_proto!(...) } } }` 包装，`crate::grpc_client::pb::magic::market::v1` 路径保持计划不变。
4. **Step 5 测试代码需 `use prost::Message;`** — `encode_to_vec`/`decode` 是 `prost::Message` trait 方法，计划草稿漏了 import（E0599）。

验证：`cargo build --lib` exit 0（4m56s 首次全量）；`cargo test --lib grpc_client::pb::` → `generated_types_roundtrip ... ok`，1 passed 0 failed。

---

### Task 2: grpc_contract/ops.rs — Operation 映射表

**Files:**
- Create: `src/grpc_contract/ops.rs`

**Interfaces:**
- Consumes: Task 1 生成代码 `crate::grpc_client::pb::magic::market::v1::Operation`
- Produces:
  - `pub fn method_name(op: Operation) -> &'static str` — proto 方法名（如 `Operation::RealtimeQuotes` → `"RealtimeQuotes"`）
  - `pub fn implemented_operations() -> Vec<Operation>` — 服务端已实现集合（24 个生产 op）
  - `pub fn is_implemented(op: Operation) -> bool`

- [x] **Step 1: 写映射表（先测后码）**

`src/grpc_contract/ops.rs`：

```rust
//! Operation ↔ proto 方法名映射 + 已实现集合。
//! 54 个 op 全部列出 (合同 market.proto 冻结); 生产未用到的 op 不进 implemented。
use crate::grpc_client::pb::magic::market::v1::Operation;

/// proto 方法名 (MarketDataService 的 RPC 名, 与 market.proto 一一对应)。
pub fn method_name(op: Operation) -> &'static str {
    use Operation::*;
    match op {
        HistoricalBars => "HistoricalBars",
        MinuteData => "MinuteData",
        RealtimeQuotes => "RealtimeQuotes",
        MoneyFlows => "MoneyFlows",
        OrderBooks => "OrderBooks",
        Auctions => "Auctions",
        Trades => "Trades",
        SecurityMetadata => "SecurityMetadata",
        GlobalIndices => "GlobalIndices",
        ForeignExchange => "ForeignExchange",
        EconomicCalendar => "EconomicCalendar",
        FuturesDelivery => "FuturesDelivery",
        ReferenceRates => "ReferenceRates",
        OfficialFxFixings => "OfficialFxFixings",
        EconomicSeries => "EconomicSeries",
        CompanyFilings => "CompanyFilings",
        GlobalNews => "GlobalNews",
        Announcements => "Announcements",
        MarketAnnouncements => "MarketAnnouncements",
        InvestorQuestions => "InvestorQuestions",
        PolicyDocuments => "PolicyDocuments",
        SecurityProfiles => "SecurityProfiles",
        FinancialStatements => "FinancialStatements",
        MarketStatistics => "MarketStatistics",
        TechnicalBars => "TechnicalBars",
        CorporateActions => "CorporateActions",
        BoardDirectory => "BoardDirectory",
        BoardConstituents => "BoardConstituents",
        BoardMemberships => "BoardMemberships",
        ResearchReports => "ResearchReports",
        ResearchDocuments => "ResearchDocuments",
        Consensus => "Consensus",
        TargetPrices => "TargetPrices",
        SemanticSearch => "SemanticSearch",
        FundFlowSeries => "FundFlowSeries",
        BoardFlows => "BoardFlows",
        MarginData => "MarginData",
        BlockTrades => "BlockTrades",
        HolderCounts => "HolderCounts",
        LockupEvents => "LockupEvents",
        DividendPlans => "DividendPlans",
        PostCloseFlows => "PostCloseFlows",
        NorthboundDaily => "NorthboundDaily",
        LimitPools => "LimitPools",
        StrongStockReasons => "StrongStockReasons",
        DragonTiger => "DragonTiger",
        MarketDragonTiger => "MarketDragonTiger",
        DragonTigerDiscovery => "DragonTigerDiscovery",
        MarketRankings => "MarketRankings",
        MarketBreadth => "MarketBreadth",
        Popularity => "Popularity",
        ConceptHits => "ConceptHits",
        OptionData => "OptionData",
        ProviderTopNRankings => "ProviderTopNRankings",
        Unspecified => "OPERATION_UNSPECIFIED",
    }
}

/// 生产实际用到的 24 个 op (spec §4.2 清单, P2 冻结)。
pub fn implemented_operations() -> Vec<Operation> {
    use Operation::*;
    vec![
        RealtimeQuotes, HistoricalBars, MinuteData, OrderBooks, MoneyFlows,
        SecurityMetadata, Announcements, GlobalNews, EconomicCalendar,
        FuturesDelivery, GlobalIndices, BoardDirectory, BoardConstituents,
        BoardFlows, LimitPools, StrongStockReasons, DragonTiger,
        MarketDragonTiger, MarketRankings, ConceptHits, Consensus,
        ResearchReports, BlockTrades, NorthboundDaily,
    ]
}

pub fn is_implemented(op: Operation) -> bool {
    implemented_operations().contains(&op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::Operation;

    #[test]
    fn method_name_covers_all_54_operations() {
        // 从 proto 的 Operation 枚举全量遍历 (0..=54), 每个都映射到非空方法名。
        for value in 0..=54 {
            if let Some(op) = Operation::from_i32(value) {
                assert!(!method_name(op).is_empty(), "op {value} 缺少方法名映射");
            }
        }
    }

    #[test]
    fn implemented_set_is_24_and_within_54() {
        assert_eq!(implemented_operations().len(), 24);
        assert!(implemented_operations()
            .iter()
            .all(|op| !matches!(op, Operation::Unspecified)));
        assert!(implemented_operations()
            .iter()
            .all(|op| !method_name(op).is_empty()));
    }

    #[test]
    fn realtime_quotes_is_implemented() {
        assert!(is_implemented(Operation::RealtimeQuotes));
        assert!(!is_implemented(Operation::OptionData));
    }
}
```

- [x] **Step 2: 跑测试确认先失败**

Run: `cargo test --lib grpc_contract::ops:: 2>&1 | tail -8`
Expected: FAIL（`method_name` 未定义）。若 pb 模块或 Operation 枚举未生成（Task 1 未完成），先回 Task 1。

- [x] **Step 3: 补 `mod ops;`**

`src/grpc_contract/mod.rs` 已有 `pub mod ops;`（Task 1 写了骨架），确认存在即可。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test --lib grpc_contract::ops:: 2>&1 | tail -5`
Expected: PASS（3 passed）。

- [x] **Step 5: Commit**

```bash
git add src/grpc_contract/ops.rs
git commit -m "feat(grpc): P0 Operation↔方法名映射表 + 24 op 已实现集合 (54 全覆盖单测)"
```

---

### Task 3: grpc_contract/schema.rs + validate.rs — schema 注册表

**Files:**
- Create: `src/grpc_contract/schema.rs`
- Create: `src/grpc_contract/validate.rs`

**Interfaces:**
- Consumes: Task 2 `method_name`/`is_implemented`；生成代码 `Operation`
- Produces:
  - `pub struct OpSchema { pub operation: Operation, pub schema_name: &'static str, pub schema_version: u32 }`
  - `pub fn schema_for(op: Operation) -> Option<&'static OpSchema>` — 已冻结 schema 的 op
  - `pub fn validate_payload(schema: &str, version: u32, data: &[u8]) -> Result<serde_json::Value, SchemaError>` — 未知 schema/version 拒绝（合同 §5）；返回解析后的 JSON
  - `pub enum SchemaError { UnknownSchema, UnsupportedVersion, NotJson }`

- [x] **Step 1: 写 schema 注册表（先测后码）**

`src/grpc_contract/schema.rs`：

```rust
//! canonical JSON schema 注册表 (合同 §5: 每方法 schema 名/版本冻结;
//! 调用方遇到未知 schema/version 必须停止解析, 不能忽略或猜字段)。
//!
//! 初始以 data_gateway 返回类型的 JSON 为准, 冻结 24 个生产 op;
//! schema 名约定: "<域>.<数据族>", 版本从 1 起。
use crate::grpc_client::pb::magic::market::v1::Operation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSchema {
    pub operation: Operation,
    pub schema_name: &'static str,
    pub schema_version: u32,
}

const SCHEMAS: &[OpSchema] = &[
    OpSchema { operation: Operation::RealtimeQuotes, schema_name: "market.realtime_quotes", schema_version: 1 },
    OpSchema { operation: Operation::HistoricalBars, schema_name: "market.historical_bars", schema_version: 1 },
    OpSchema { operation: Operation::MinuteData, schema_name: "market.minute_data", schema_version: 1 },
    OpSchema { operation: Operation::OrderBooks, schema_name: "market.order_books", schema_version: 1 },
    OpSchema { operation: Operation::MoneyFlows, schema_name: "market.money_flows", schema_version: 1 },
    OpSchema { operation: Operation::SecurityMetadata, schema_name: "market.security_metadata", schema_version: 1 },
    OpSchema { operation: Operation::Announcements, schema_name: "news.announcements", schema_version: 1 },
    OpSchema { operation: Operation::GlobalNews, schema_name: "news.global_news", schema_version: 1 },
    OpSchema { operation: Operation::EconomicCalendar, schema_name: "market.economic_calendar", schema_version: 1 },
    OpSchema { operation: Operation::FuturesDelivery, schema_name: "market.futures_delivery", schema_version: 1 },
    OpSchema { operation: Operation::GlobalIndices, schema_name: "market.global_indices", schema_version: 1 },
    OpSchema { operation: Operation::BoardDirectory, schema_name: "board.directory", schema_version: 1 },
    OpSchema { operation: Operation::BoardConstituents, schema_name: "board.constituents", schema_version: 1 },
    OpSchema { operation: Operation::BoardFlows, schema_name: "board.flows", schema_version: 1 },
    OpSchema { operation: Operation::LimitPools, schema_name: "market.limit_pools", schema_version: 1 },
    OpSchema { operation: Operation::StrongStockReasons, schema_name: "market.strong_stock_reasons", schema_version: 1 },
    OpSchema { operation: Operation::DragonTiger, schema_name: "market.dragon_tiger", schema_version: 1 },
    OpSchema { operation: Operation::MarketDragonTiger, schema_name: "market.market_dragon_tiger", schema_version: 1 },
    OpSchema { operation: Operation::MarketRankings, schema_name: "market.market_rankings", schema_version: 1 },
    OpSchema { operation: Operation::ConceptHits, schema_name: "market.concept_hits", schema_version: 1 },
    OpSchema { operation: Operation::Consensus, schema_name: "market.consensus", schema_version: 1 },
    OpSchema { operation: Operation::ResearchReports, schema_name: "research.reports", schema_version: 1 },
    OpSchema { operation: Operation::BlockTrades, schema_name: "market.block_trades", schema_version: 1 },
    OpSchema { operation: Operation::NorthboundDaily, schema_name: "market.northbound_daily", schema_version: 1 },
];

pub fn schema_for(op: Operation) -> Option<&'static OpSchema> {
    SCHEMAS.iter().find(|s| s.operation == op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_implemented_op_has_frozen_schema() {
        // 24 个已实现 op 全部有 schema (spec 验收标准 3)。
        assert_eq!(SCHEMAS.len(), 24);
        for op in crate::grpc_contract::ops::implemented_operations() {
            assert!(schema_for(op).is_some(), "op {op:?} 缺 schema");
        }
    }

    #[test]
    fn schema_names_are_unique() {
        let mut names: Vec<&str> = SCHEMAS.iter().map(|s| s.schema_name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SCHEMAS.len());
    }
}
```

- [x] **Step 2: 写 validate.rs（先测后码）**

`src/grpc_contract/validate.rs`：

```rust
//! canonical payload 校验 (合同 §5: 未知 schema/version 必须停止解析)。
use crate::grpc_contract::schema::schema_for;
use crate::grpc_client::pb::magic::market::v1::Operation;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SchemaError {
    #[error("未知 schema: {0} (不允许忽略或猜字段)")]
    UnknownSchema(String),
    #[error("schema {0} 版本不支持: {1}")]
    UnsupportedVersion(String, u32),
    #[error("payload 不是合法 UTF-8 JSON: {0}")]
    NotJson(String),
}

/// 校验 schema/version 并解析 data 为 JSON。失败必须拒绝, 不返回部分结果。
pub fn validate_payload(
    operation: Operation,
    schema: &str,
    version: u32,
    data: &[u8],
) -> Result<serde_json::Value, SchemaError> {
    let frozen = schema_for(operation)
        .ok_or_else(|| SchemaError::UnknownSchema(schema.to_string()))?;
    if frozen.schema_name != schema {
        return Err(SchemaError::UnknownSchema(schema.to_string()));
    }
    if frozen.schema_version != version {
        return Err(SchemaError::UnsupportedVersion(schema.to_string(), version));
    }
    serde_json::from_slice(data).map_err(|e| SchemaError::NotJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::Operation;

    #[test]
    fn rejects_unknown_schema() {
        let err = validate_payload(Operation::RealtimeQuotes, "not.a.schema", 1, b"[]")
            .unwrap_err();
        assert_eq!(err, SchemaError::UnknownSchema("not.a.schema".to_string()));
    }

    #[test]
    fn rejects_wrong_schema_for_operation() {
        // Announcements 的 schema 名不能用于 RealtimeQuotes。
        let err = validate_payload(
            Operation::RealtimeQuotes,
            "news.announcements",
            1,
            b"[]",
        )
        .unwrap_err();
        assert!(matches!(err, SchemaError::UnknownSchema(_)));
    }

    #[test]
    fn rejects_unsupported_version() {
        let err = validate_payload(Operation::RealtimeQuotes, "market.realtime_quotes", 99, b"[]")
            .unwrap_err();
        assert_eq!(
            err,
            SchemaError::UnsupportedVersion("market.realtime_quotes".to_string(), 99)
        );
    }

    #[test]
    fn rejects_non_json_data() {
        let err = validate_payload(Operation::RealtimeQuotes, "market.realtime_quotes", 1, b"not json")
            .unwrap_err();
        assert!(matches!(err, SchemaError::NotJson(_)));
    }

    #[test]
    fn parses_valid_json() {
        let value = validate_payload(
            Operation::RealtimeQuotes,
            "market.realtime_quotes",
            1,
            br#"[{"code":"600519"}]"#,
        )
        .unwrap();
        assert_eq!(value[0]["code"], "600519");
    }
}
```

- [x] **Step 3: 补 mod 声明 + 跑测试**

`src/grpc_contract/mod.rs` 已声明 `pub mod schema; pub mod validate;`（Task 1 骨架），确认。Run: `cargo test --lib grpc_contract:: 2>&1 | tail -8`
Expected: PASS（schema 2 + validate 5 通过；Task 2 的 3 个也过）。`cargo build --lib` exit 0。

- [x] **Step 4: Commit**

```bash
git add src/grpc_contract/schema.rs src/grpc_contract/validate.rs
git commit -m "feat(grpc): P0 schema 注册表 (24 op 冻结) + canonical payload 校验 (未知 schema/version 拒绝)"
```

---

### Task 4: grpc_client/envelope.rs — request_id + QueryRequest/QueryResponse

**Files:**
- Create: `src/grpc_client/envelope.rs`

**Interfaces:**
- Consumes: Task 3 `schema_for`
- Produces:
  - `pub fn new_request_id() -> String` — `{unix_ms}-{pid}-{counter}` 非空唯一
  - `pub fn build_query_request(op: Operation, payload: serde_json::Value) -> Result<QueryRequest, EnvelopeError>` — 从注册表取 schema 名/版本，`preferred_provider` 恒为空
  - `pub struct QueryResult { pub admission: AdmissionState, pub selected_provider: String, pub batch_id: String, pub complete: bool, pub observed_at: String, pub source_at: String, pub records: Vec<CanonicalPayload> }`
  - `pub fn parse_query_response(expected_request_id: &str, resp: QueryResponse) -> Result<QueryResult, EnvelopeError>` — request_id 必须匹配
  - `pub enum EnvelopeError { RequestIdMismatch, MissingContext, InvalidPayload }`

- [x] **Step 1: 写 envelope.rs（先测后码）**

```rust
//! QueryRequest/QueryResponse 信封 (合同 §5/§6)。
//! request_id: 调用方生成的非空唯一请求 ID; 同一业务重试保留原 ID。
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, CanonicalPayload, QueryRequest, QueryResponse, RequestContext,
};
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::grpc_contract::schema::schema_for;
use std::sync::atomic::{AtomicU64, Ordering};

const PROTOCOL_VERSION: u32 = 1;
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn new_request_id() -> String {
    let ms = chrono::Utc::now().timestamp_millis();
    let pid = std::process::id();
    let n = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{ms}-{pid}-{n}")
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EnvelopeError {
    #[error("request_id 不匹配: 期望 {0} 实际 {1}")]
    RequestIdMismatch(String, String),
    #[error("响应缺 request_id")]
    MissingRequestId,
    #[error("schema 未冻结, 无法构造请求")]
    SchemaNotFrozen,
    #[error("payload 序列化失败: {0}")]
    Serialize(String),
}

pub fn build_query_request(
    op: Operation,
    payload: serde_json::Value,
) -> Result<QueryRequest, EnvelopeError> {
    let schema = schema_for(op).ok_or(EnvelopeError::SchemaNotFrozen)?;
    let data = serde_json::to_vec(&payload)
        .map_err(|e| EnvelopeError::Serialize(e.to_string()))?;
    Ok(QueryRequest {
        context: Some(RequestContext {
            protocol_version: PROTOCOL_VERSION,
            request_id: new_request_id(),
        }),
        // 合同 §5: 普通调用保持 preferred_provider 为空, 由服务端 Composition 选择。
        preferred_provider: String::new(),
        payload: Some(CanonicalPayload {
            schema: schema.schema_name.to_string(),
            schema_version: schema.schema_version,
            content_type: "application/json; charset=utf-8".to_string(),
            data,
        }),
    })
}

pub struct QueryResult {
    pub admission: AdmissionState,
    pub selected_provider: String,
    pub batch_id: String,
    pub complete: bool,
    pub observed_at: String,
    pub source_at: String,
    pub records: Vec<CanonicalPayload>,
}

pub fn parse_query_response(
    expected_request_id: &str,
    resp: QueryResponse,
) -> Result<QueryResult, EnvelopeError> {
    if resp.request_id.is_empty() {
        return Err(EnvelopeError::MissingRequestId);
    }
    if resp.request_id != expected_request_id {
        return Err(EnvelopeError::RequestIdMismatch(
            expected_request_id.to_string(),
            resp.request_id,
        ));
    }
    Ok(QueryResult {
        admission: AdmissionState::from_i32(resp.admission)
            .unwrap_or(AdmissionState::Unspecified),
        selected_provider: resp.selected_provider,
        batch_id: resp.batch_id,
        complete: resp.complete,
        observed_at: resp.observed_at,
        source_at: resp.source_at,
        records: resp.records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_unique_and_nonempty() {
        let a = new_request_id();
        let b = new_request_id();
        assert!(!a.is_empty() && !b.is_empty());
        assert_ne!(a, b);
    }

    #[test]
    fn builds_query_request_with_frozen_schema() {
        let req = build_query_request(
            Operation::RealtimeQuotes,
            serde_json::json!({"codes": ["600519"]}),
        )
        .unwrap();
        let ctx = req.context.unwrap();
        assert_eq!(ctx.protocol_version, 1);
        assert!(!ctx.request_id.is_empty());
        assert_eq!(req.preferred_provider, "");
        let payload = req.payload.unwrap();
        assert_eq!(payload.schema, "market.realtime_quotes");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
    }

    #[test]
    fn rejects_query_request_for_unfrozen_schema() {
        let err = build_query_request(
            Operation::OptionData, // 不在 SCHEMAS (未实现 op)
            serde_json::json!({}),
        )
        .unwrap_err();
        assert_eq!(err, EnvelopeError::SchemaNotFrozen);
    }

    #[test]
    fn parses_query_response_with_matching_request_id() {
        let resp = QueryResponse {
            request_id: "r-1".to_string(),
            operation: 3,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "t1".to_string(),
            source_at: "t2".to_string(),
            records: vec![],
        };
        let result = parse_query_response("r-1", resp).unwrap();
        assert_eq!(result.admission, AdmissionState::Admitted);
        assert!(result.complete);
        assert_eq!(result.selected_provider, "tdx-dev");
    }

    #[test]
    fn rejects_mismatched_request_id() {
        let resp = QueryResponse {
            request_id: "other".to_string(),
            operation: 3,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
        };
        let err = parse_query_response("r-1", resp).unwrap_err();
        assert!(matches!(err, EnvelopeError::RequestIdMismatch(_, _)));
    }

    #[test]
    fn rejects_missing_request_id() {
        let resp = QueryResponse {
            request_id: String::new(),
            operation: 3,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
        };
        assert_eq!(parse_query_response("r-1", resp).unwrap_err(), EnvelopeError::MissingRequestId);
    }
}
```

- [x] **Step 2: 跑测试**

Run: `cargo test --lib grpc_client::envelope:: 2>&1 | tail -6`
Expected: PASS（6 passed）。`cargo build --lib` exit 0。

- [x] **Step 3: Commit**

```bash
git add src/grpc_client/envelope.rs
git commit -m "feat(grpc): P0 request_id + QueryRequest/QueryResponse 信封 (匹配校验/冻结 schema)"
```

---

### Task 5: grpc_client/errors.rs + retry.rs — 错误映射与重试

**Files:**
- Create: `src/grpc_client/errors.rs`
- Create: `src/grpc_client/retry.rs`

**Interfaces:**
- Consumes: 无（独立；后续 client.rs 使用）
- Produces:
  - `pub enum GrpcError { InvalidArgument, Unauthenticated, PermissionDenied, Unimplemented, ResourceExhausted, DeadlineExceeded, Unavailable, FailedPrecondition, Internal, Unknown { details: ErrorDetail } }`（`impl From<tonic::Status>`）
  - `pub struct ErrorDetail { pub code: String, pub request_id: Option<String>, pub operation: Option<i32>, pub provider: Option<String>, pub reason_code: Option<String>, pub retryable: Option<bool> }`
  - `pub enum RetryDecision { RetryBackoff, RetryBounded, NoRetry }` + `pub fn retry_decision(err: &GrpcError) -> RetryDecision`（§10 表）
  - `pub struct RetryPolicy { pub max_attempts: u32, pub base_delay_ms: u64, pub max_delay_ms: u64, pub jitter_ms: u64 }` + `impl Default`（4 次 / 1s / 60s / 200ms）+ `pub fn backoff(&self, attempt: u32) -> Duration`（指数退避 1s→60s 封顶）

- [x] **Step 1: 写 errors.rs（先测后码）**

```rust
//! gRPC status code → 项目错误类型 (合同 §10 错误映射表)。
//! 不依赖自然语言 message 做程序分支; ErrorDetail 从 status details 解码。

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum GrpcError {
    #[error("请求参数错误 (不重试)")]
    InvalidArgument,
    #[error("认证失败 (刷新凭据)")]
    Unauthenticated,
    #[error("无权限调用该能力 (停止调用)")]
    PermissionDenied,
    #[error("能力未准入或不支持 (不重试)")]
    Unimplemented,
    #[error("资源受限 (退避; 流消费者记录 gap)")]
    ResourceExhausted,
    #[error("超时 (有界重试, 保留原 request_id)")]
    DeadlineExceeded,
    #[error("服务不可用 (指数退避, 重新检查 health/capabilities)")]
    Unavailable,
    #[error("数据完整性/连续性失败 (不能当空成功)")]
    FailedPrecondition,
    #[error("服务端内部错误 (记录 request_id, 停止无界重试)")]
    Internal,
    #[error("未知错误 (code={code})", code = details.code)]
    Unknown { details: ErrorDetail },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ErrorDetail {
    pub code: String,
    pub request_id: Option<String>,
    pub operation: Option<i32>,
    pub provider: Option<String>,
    pub reason_code: Option<String>,
    pub retryable: Option<bool>,
}

impl From<tonic::Status> for GrpcError {
    fn from(status: tonic::Status) -> Self {
        // 尝试解码安全 ErrorDetail (合同 §10: request ID/operation/provider/reason code/retryable)。
        let detail = status
            .details()
            .get(0)
            .and_then(|any| {
                // 只解析 proto ErrorDetail; 失败则忽略, 用 code 分支即可。
                any.downcast_ref::<crate::grpc_client::pb::magic::market::v1::ErrorDetail>()
                    .map(|d| ErrorDetail {
                        code: status.code().to_string(),
                        request_id: if d.request_id.is_empty() { None } else { Some(d.request_id.clone()) },
                        operation: Some(d.operation),
                        provider: if d.provider.is_empty() { None } else { Some(d.provider.clone()) },
                        reason_code: if d.reason_code.is_empty() { None } else { Some(d.reason_code.clone()) },
                        retryable: Some(d.retryable),
                    })
                    .or_else(|| Some(ErrorDetail { code: status.code().to_string(), ..Default::default() }))
            })
            .unwrap_or_else(|| ErrorDetail { code: status.code().to_string(), ..Default::default() });

        match status.code() {
            tonic::Code::InvalidArgument => GrpcError::InvalidArgument,
            tonic::Code::Unauthenticated => GrpcError::Unauthenticated,
            tonic::Code::PermissionDenied => GrpcError::PermissionDenied,
            tonic::Code::Unimplemented => GrpcError::Unimplemented,
            tonic::Code::ResourceExhausted => GrpcError::ResourceExhausted,
            tonic::Code::DeadlineExceeded => GrpcError::DeadlineExceeded,
            tonic::Code::Unavailable => GrpcError::Unavailable,
            tonic::Code::FailedPrecondition => GrpcError::FailedPrecondition,
            tonic::Code::Internal => GrpcError::Internal,
            _ => GrpcError::Unknown { details: detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn maps_all_contract_codes() {
        let cases = [
            (Code::InvalidArgument, GrpcError::InvalidArgument),
            (Code::Unauthenticated, GrpcError::Unauthenticated),
            (Code::PermissionDenied, GrpcError::PermissionDenied),
            (Code::Unimplemented, GrpcError::Unimplemented),
            (Code::ResourceExhausted, GrpcError::ResourceExhausted),
            (Code::DeadlineExceeded, GrpcError::DeadlineExceeded),
            (Code::Unavailable, GrpcError::Unavailable),
            (Code::FailedPrecondition, GrpcError::FailedPrecondition),
            (Code::Internal, GrpcError::Internal),
            (Code::Unknown, GrpcError::Unknown { details: ErrorDetail { code: "Unknown".into(), ..Default::default() } }),
        ];
        for (code, expected) in cases {
            let status = tonic::Status::new(code, "msg");
            assert_eq!(GrpcError::from(status), expected, "code {code:?}");
        }
    }
}
```

- [x] **Step 2: 写 retry.rs（先测后码）**

```rust
//! 有界重试与指数退避 (合同 §10)。
//! UNAVAILABLE → 指数退避 + 重查 health; DEADLINE_EXCEEDED → 有界重试保留原 request_id。
use crate::grpc_client::errors::GrpcError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RetryDecision {
    RetryBackoff,   // UNAVAILABLE: 指数退避
    RetryBounded,   // DEADLINE_EXCEEDED: 固定次数
    NoRetry,
}

/// §10 表: 每个错误码的重试决策。
pub fn retry_decision(err: &GrpcError) -> RetryDecision {
    match err {
        GrpcError::Unavailable => RetryDecision::RetryBackoff,
        GrpcError::DeadlineExceeded => RetryDecision::RetryBounded,
        _ => RetryDecision::NoRetry,
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,      // 总尝试次数 (含首次)
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,      // 首次 + 3 次退避
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
            jitter_ms: 200,
        }
    }
}

impl RetryPolicy {
    /// 第 attempt 次 (1-based) 重试前的等待时长, 指数退避 + jitter。
    pub fn backoff(&self, attempt: u32) -> std::time::Duration {
        let exponent = attempt.saturating_sub(1).min(6);
        let base = self.base_delay_ms << exponent;
        let capped = base.min(self.max_delay_ms);
        std::time::Duration::from_millis(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_follows_contract_table() {
        assert_eq!(retry_decision(&GrpcError::Unavailable), RetryDecision::RetryBackoff);
        assert_eq!(retry_decision(&GrpcError::DeadlineExceeded), RetryDecision::RetryBounded);
        assert_eq!(retry_decision(&GrpcError::InvalidArgument), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::Unauthenticated), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::PermissionDenied), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::Unimplemented), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::ResourceExhausted), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::FailedPrecondition), RetryDecision::NoRetry);
        assert_eq!(retry_decision(&GrpcError::Internal), RetryDecision::NoRetry);
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let p = RetryPolicy::default();
        assert!(p.backoff(1) < p.backoff(2));
        assert!(p.backoff(3) < p.backoff(4));
        assert!(p.backoff(10) <= std::time::Duration::from_millis(60_000));
    }
}
```

- [x] **Step 3: 跑测试**

Run: `cargo test --lib grpc_client::errors:: grpc_client::retry:: 2>&1 | tail -6`
Expected: PASS（2 passed）。`cargo build --lib` exit 0。

- [x] **Step 4: Commit**

```bash
git add src/grpc_client/errors.rs src/grpc_client/retry.rs
git commit -m "feat(grpc): P0 gRPC 错误码映射 (§10 表) + 指数退避/有界重试策略"
```

---

### Task 6: grpc_client/auth.rs — Bearer metadata

**Files:**
- Create: `src/grpc_client/auth.rs`

**Interfaces:**
- Consumes: 无
- Produces: `pub fn attach_bearer<T>(request: &mut tonic::Request<T>) -> Result<(), AuthError>` — 从 `GRPC_MARKET_TOKEN` 读 token，注入 `authorization: Bearer <token>`；未设置 → 不注入（dev 服务端接受）；`pub enum AuthError { InvalidTokenValue }`

- [x] **Step 1: 写 auth.rs（先测后码）**

```rust
//! 认证 (合同 §4): 业务客户端通过 gRPC metadata 发送 `authorization: Bearer <token>`。
//! token 只进 metadata, 不进请求体/URL/日志。
use tonic::metadata::MetadataValue;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AuthError {
    #[error("GRPC_MARKET_TOKEN 包含非法字符, 无法注入 metadata")]
    InvalidTokenValue,
}

/// 从 GRPC_MARKET_TOKEN 读 token 并注入 authorization metadata。
/// token 未设置 → 不注入 (dev 服务端明文接受; 真实服务端对接时必须设置)。
pub fn attach_bearer<T>(request: &mut tonic::Request<T>) -> Result<(), AuthError> {
    let Ok(token) = std::env::var("GRPC_MARKET_TOKEN") else {
        return Ok(());
    };
    let value = format!("Bearer {token}");
    let metadata = MetadataValue::try_from(value.as_str())
        .map_err(|_| AuthError::InvalidTokenValue)?;
    request.metadata_mut().insert("authorization", metadata);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_token<R>(token: &str, f: impl FnOnce()) {
        // 环境变量是进程级的: 串行跑本模块测试即可, 不与其他模块并行依赖。
        unsafe {
            std::env::set_var("GRPC_MARKET_TOKEN", token);
        }
        f();
        unsafe {
            std::env::remove_var("GRPC_MARKET_TOKEN");
        }
    }

    #[test]
    fn injects_bearer_when_token_set() {
        with_token("secret-token", || {
            let mut req = tonic::Request::new(());
            attach_bearer(&mut req).unwrap();
            let auth = req.metadata().get("authorization").unwrap().to_str().unwrap();
            assert_eq!(auth, "Bearer secret-token");
        });
    }

    #[test]
    fn no_op_when_token_unset() {
        let mut req = tonic::Request::new(());
        attach_bearer(&mut req).unwrap();
        assert!(req.metadata().get("authorization").is_none());
    }
}
```

- [x] **Step 2: 跑测试**

Run: `cargo test --lib grpc_client::auth:: 2>&1 | tail -5`
Expected: PASS（2 passed）。`cargo build --lib` exit 0。

- [x] **Step 3: Commit**

```bash
git add src/grpc_client/auth.rs
git commit -m "feat(grpc): P0 Bearer token 注入 (GRPC_MARKET_TOKEN, 只进 metadata)"
```

---

### Task 7: grpc_client/client.rs — GrpcMarketClient

**Files:**
- Create: `src/grpc_client/client.rs`

**Interfaces:**
- Consumes: Task 2 `method_name`/`is_implemented`、Task 3 `schema_for`、Task 4 `build_query_request`/`parse_query_response`/`QueryResult`/`new_request_id`、Task 5 `GrpcError`/`RetryPolicy`/`retry_decision`、Task 6 `attach_bearer`
- Produces:
  - `pub struct GrpcMarketClient { data: MarketDataServiceClient<Channel>, system: SystemServiceClient<Channel>, events: MarketEventServiceClient<Channel>, retry: RetryPolicy }`
  - `pub async fn connect(addr: &str) -> Result<Self, GrpcError>`
  - `pub async fn get_health(&mut self) -> Result<HealthResponse, GrpcError>`
  - `pub async fn get_capabilities(&mut self) -> Result<Vec<Capability>, GrpcError>`
  - `pub async fn query(&mut self, op: Operation, payload: serde_json::Value) -> Result<QueryResult, GrpcError>` — 未实现 op 客户端直接 `GrpcError::Unimplemented`（不发起调用）；§10 重试保留原 request_id
  - `pub async fn subscribe(&mut self, filter: EventFilter, after: Option<EventCursor>) -> Result<tonic::Streaming<MarketEventEnvelope>, GrpcError>`（Task 11 消费）

- [x] **Step 1: 写 client.rs（先测后码）**

```rust
//! GrpcMarketClient: 54 op 的 gRPC 查询客户端 (合同 §5-§7)。
//! 启动后应先调 GetCapabilities (合同 §7: RPC 存在 ≠ 能力准入)。
use crate::grpc_client::auth::attach_bearer;
use crate::grpc_client::envelope::{build_query_request, parse_query_response, QueryResult};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient, market_event_service_client::MarketEventServiceClient,
    system_service_client::SystemServiceClient, CapabilitiesRequest, EventCursor, EventFilter,
    HealthRequest, Operation, SubscribeRequest,
};
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};
use std::time::Duration;
use tonic::transport::Channel;

pub struct GrpcMarketClient {
    data: MarketDataServiceClient<Channel>,
    system: SystemServiceClient<Channel>,
    events: MarketEventServiceClient<Channel>,
    retry: RetryPolicy,
}

impl GrpcMarketClient {
    pub async fn connect(addr: &str) -> Result<Self, GrpcError> {
        let channel = Channel::from_shared(addr.to_string())
            .map_err(|_| GrpcError::InvalidArgument)?
            // 合同 §12: 为 unary 和 stream 分别设置 deadline/keepalive。
            .timeout(Duration::from_secs(15))
            .connect()
            .await
            .map_err(|_| GrpcError::Unavailable)?;
        Ok(Self {
            data: MarketDataServiceClient::new(channel.clone()),
            system: SystemServiceClient::new(channel.clone()),
            events: MarketEventServiceClient::new(channel),
            retry: RetryPolicy::default(),
        })
    }

    pub async fn get_health(&mut self) -> Result<crate::grpc_client::pb::magic::market::v1::HealthResponse, GrpcError> {
        let mut req = tonic::Request::new(HealthRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
        });
        attach_bearer(&mut req)?;
        let resp = self.system.get_health(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }

    pub async fn get_capabilities(
        &mut self,
    ) -> Result<Vec<crate::grpc_client::pb::magic::market::v1::Capability>, GrpcError> {
        let mut req = tonic::Request::new(CapabilitiesRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
        });
        attach_bearer(&mut req)?;
        let resp = self.system.get_capabilities(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner().capabilities)
    }

    /// 按 §10 重试语义执行一次查询。未实现 op 在客户端拦截 (不发起调用)。
    pub async fn query(
        &mut self,
        op: Operation,
        payload: serde_json::Value,
    ) -> Result<QueryResult, GrpcError> {
        if !crate::grpc_contract::ops::is_implemented(op) {
            return Err(GrpcError::Unimplemented);
        }
        let request = build_query_request(op, payload)?;
        let request_id = request
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_default();

        let mut attempt: u32 = 1;
        loop {
            let outcome = self.data_call(op, request.clone()).await;
            match outcome {
                Ok(resp) => return parse_query_response(&request_id, resp).map_err(|e| {
                    // 信封错误 (request_id 失配等) 映射为 Unknown, 保留 details。
                    GrpcError::Unknown {
                        details: crate::grpc_client::errors::ErrorDetail {
                            code: "envelope".to_string(),
                            ..Default::default()
                        },
                    }
                }),
                Err(err) => match retry_decision(&err) {
                    RetryDecision::RetryBackoff | RetryDecision::RetryBounded
                        if attempt < self.retry.max_attempts =>
                    {
                        tokio::time::sleep(self.retry.backoff(attempt)).await;
                        attempt += 1;
                    }
                    _ => return Err(err),
                },
            }
        }
    }

    /// Operation → MarketDataService 方法调用 (实现 op 的 match; 其余已由 is_implemented 拦截)。
    async fn data_call(
        &mut self,
        op: Operation,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::QueryResponse, GrpcError> {
        let mut req = tonic::Request::new(request);
        attach_bearer(&mut req)?;
        let resp = match op {
            Operation::RealtimeQuotes => self.data.realtime_quotes(req).await,
            Operation::HistoricalBars => self.data.historical_bars(req).await,
            Operation::MinuteData => self.data.minute_data(req).await,
            Operation::OrderBooks => self.data.order_books(req).await,
            Operation::MoneyFlows => self.data.money_flows(req).await,
            Operation::SecurityMetadata => self.data.security_metadata(req).await,
            Operation::Announcements => self.data.announcements(req).await,
            Operation::GlobalNews => self.data.global_news(req).await,
            Operation::EconomicCalendar => self.data.economic_calendar(req).await,
            Operation::FuturesDelivery => self.data.futures_delivery(req).await,
            Operation::GlobalIndices => self.data.global_indices(req).await,
            Operation::BoardDirectory => self.data.board_directory(req).await,
            Operation::BoardConstituents => self.data.board_constituents(req).await,
            Operation::BoardFlows => self.data.board_flows(req).await,
            Operation::LimitPools => self.data.limit_pools(req).await,
            Operation::StrongStockReasons => self.data.strong_stock_reasons(req).await,
            Operation::DragonTiger => self.data.dragon_tiger(req).await,
            Operation::MarketDragonTiger => self.data.market_dragon_tiger(req).await,
            Operation::MarketRankings => self.data.market_rankings(req).await,
            Operation::ConceptHits => self.data.concept_hits(req).await,
            Operation::Consensus => self.data.consensus(req).await,
            Operation::ResearchReports => self.data.research_reports(req).await,
            Operation::BlockTrades => self.data.block_trades(req).await,
            Operation::NorthboundDaily => self.data.northbound_daily(req).await,
            _ => return Err(GrpcError::Unimplemented), // 防御: is_implemented 已拦截
        };
        match resp {
            Ok(r) => Ok(r.into_inner()),
            Err(status) => Err(GrpcError::from(status)),
        }
    }

    pub async fn subscribe(
        &mut self,
        filter: EventFilter,
        after: Option<EventCursor>,
    ) -> Result<tonic::Streaming<crate::grpc_client::pb::magic::market::v1::MarketEventEnvelope>, GrpcError> {
        let mut req = tonic::Request::new(SubscribeRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
            filter: Some(filter),
            after,
        });
        attach_bearer(&mut req)?;
        let resp = self.events.subscribe(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{
        market_data_service_server::{MarketDataService, MarketDataServiceServer},
        system_service_server::{SystemService, SystemServiceServer},
        CanonicalPayload, CapabilitiesRequest, CapabilitiesResponse, HealthResponse, QueryResponse,
    };
    use tonic::{Request, Response, Status};

    struct MockSystem;
    #[tonic::async_trait]
    impl SystemService for MockSystem {
        async fn get_health(&self, _req: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
            Ok(Response::new(HealthResponse { request_id: "h-1".into(), live: true, ready: true, state: "RUNNING".into() }))
        }
        async fn get_capabilities(&self, _req: Request<CapabilitiesRequest>) -> Result<Response<CapabilitiesResponse>, Status> {
            Ok(Response::new(CapabilitiesResponse { request_id: "c-1".into(), capabilities: vec![] }))
        }
    }

    struct MockData;
    #[tonic::async_trait]
    impl MarketDataService for MockData {
        async fn realtime_quotes(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
            let inner = req.into_inner();
            let request_id = inner.context.unwrap().request_id;
            Ok(Response::new(QueryResponse {
                request_id,
                operation: Operation::RealtimeQuotes as i32,
                admission: 1, // ADMITTED
                selected_provider: "mock".into(),
                batch_id: "mock-b1".into(),
                complete: true,
                observed_at: "2026-08-13T10:00:00+08:00".into(),
                source_at: "2026-08-13T10:00:00+08:00".into(),
                records: vec![CanonicalPayload {
                    schema: "market.realtime_quotes".into(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".into(),
                    data: br#"[{"code":"600519","name":"贵州茅台"}]"#.to_vec(),
                }],
            }))
        }
    }
    // 注: tonic 生成的 MarketDataService trait 共 54 个方法, 全部必须实现。
    // 上面只写了 realtime_quotes; 其余 53 个用相同模式补 `Err(Status::unimplemented("..."))` 桩,
    // 方法名 = proto RPC 名 camelCase (historical_bars / minute_data / ... / provider_top_n_rankings)。
    // 以 cargo build 报错列出的缺失方法为准逐一补齐, 勿改签名。

    async fn spawn_mock() -> String {
        let addr = "127.0.0.1:0";
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let local = listener.local_addr().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(SystemServiceServer::new(MockSystem))
                .add_service(MarketDataServiceServer::new(MockData))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        format!("http://{local}")
    }

    #[tokio::test]
    async fn query_realtime_quotes_roundtrip() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        let result = client
            .query(Operation::RealtimeQuotes, serde_json::json!({"codes": ["600519"]}))
            .await
            .unwrap();
        assert_eq!(result.admission.to_string(), "ADMITTED".to_string());
        assert!(result.complete);
        assert_eq!(result.records.len(), 1);
        let payload = &result.records[0];
        assert_eq!(payload.schema, "market.realtime_quotes");
        let parsed: serde_json::Value = serde_json::from_slice(&payload.data).unwrap();
        assert_eq!(parsed[0]["code"], "600519");
    }

    #[tokio::test]
    async fn query_unimplemented_op_returns_unimplemented() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        // OptionData 不在 implemented 集合 → 客户端直接拦截。
        let err = client.query(Operation::OptionData, serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, GrpcError::Unimplemented));
    }
}
```

- [x] **Step 2: 跑测试**

Run: `cargo test --lib grpc_client::client:: 2>&1 | tail -8`
Expected: PASS（2 passed）。若 tonic trait 方法签名报错（缺方法 / 方法名不同），按 Step 1 注释的补桩规则补齐后重跑。`cargo build --lib` exit 0。

- [x] **Step 3: Commit**

```bash
git add src/grpc_client/client.rs
git commit -m "feat(grpc): P0 GrpcMarketClient (query 24 op + health/capabilities/subscribe + §10 重试)"
```

---

### Task 8: grpc_server 库模块 + 薄二进制 + SystemService + fixture 模式 + 集成测试（P1 交付）

**Files:**
- Create: `src/grpc_server/mod.rs`（ServerConfig / start() / ServerState / SystemService impl）
- Create: `src/grpc_server/handlers.rs`（MarketDataService impl + fixture 分支）
- Create: `src/grpc_server/delegate.rs`（真实路径委托 data_gateway，Task 9/10 补全）
- Create: `src/grpc_server/fixture.rs`（fixture 数据，6 个代表 op）
- Create: `src/grpc_server/events.rs`（空骨架：仅文档注释，Task 11 填充全部实现）
- Create: `src/bin/grpc_market_server.rs`（薄入口）
- Create: `tests/grpc_channel_e2e.rs`（集成测试）
- Modify: `src/lib.rs`（加 `pub mod grpc_server;`）

**Interfaces:**
- Consumes: Task 1 生成代码（服务 trait）、Task 2 `implemented_operations`、Task 3 `schema_for`
- Produces:
  - `pub struct ServerConfig { pub fixture_mode: bool, pub shadow_events: bool, pub port: u16, pub instruments: Vec<String> }`（`impl Default` 读 env：`GRPC_GATEWAY_TEST_FIXTURE=1`、`GRPC_EVENTS_SHADOW=1`、`GRPC_MARKET_PORT`（默认 18082）、`STOCK_LIST` 逗号分割）
  - `pub async fn start(config: ServerConfig) -> anyhow::Result<(std::net::SocketAddr, tokio::task::JoinHandle<Result<(), tonic::transport::Error>>)>` — 端口 0 时绑定随机端口并返回实际地址
  - `pub struct ServerState { pub generation: String, pub sequence: AtomicU64, pub shadow_events: bool }`
  - `pub(crate) mod delegate { pub struct Fetched { pub data: Vec<u8>, pub source_at: String }; pub fn fetch(op, schema) -> Result<Fetched, String> }`
  - `pub(crate) fn fixture_response(op: Operation, request_schema: &str, request_version: u32) -> Option<QueryResponse>`

- [x] **Step 1: 写 ServerConfig + ServerState + start() + SystemService**

先建 `src/grpc_server/events.rs` 空骨架（Task 11 填充全部实现）：

```rust
//! TDX 异动事件生成器 (合同 §8: price/volume/amount/status/reset 事件;
//! cursor generation+sequence 单调递增; UNADMITTED 影子事件必须显式隔离)。
//! Task 11 实现 diff 检测器 + EventHub + MarketEventService (Subscribe/Replay/GetListenerStatus)。
```

`src/grpc_server/mod.rs`：

```rust
//! gRPC 服务端库模块 (mock 服务端, 方案 A: handler 委托 data_gateway)。
//! 薄二进制 src/bin/grpc_market_server.rs 只负责读配置 + start()。
//! fixture_mode=true 时 handler 返回 fixture 数据 (离线确定性测试);
//! 生产/手工运行 fixture_mode=false 走真实 data_gateway → magic-* crates。
pub mod delegate;
pub mod events;
pub mod fixture;
pub mod handlers;

use crate::grpc_client::pb::magic::market::v1::{
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, HealthRequest,
    HealthResponse, ListenerStatusRequest, ListenerStatusResponse,
};
use crate::grpc_contract::ops::implemented_operations;
use std::sync::atomic::AtomicU64;
use tonic::{Request, Response, Status};

pub struct ServerState {
    /// 服务端进程代次 (合同 §8: generation 改变 = 连续性重建, 不可跨代拼接)。
    pub generation: String,
    pub sequence: AtomicU64,
    pub shadow_events: bool,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// fixture 模式: handler 返回 fixture 数据, 不连真实 provider。
    pub fixture_mode: bool,
    /// 事件标 UNADMITTED (影子模式, 测试影子隔离用)。
    pub shadow_events: bool,
    /// 监听端口; 0 = 随机端口 (集成测试用)。
    pub port: u16,
    /// 事件轮询的标的 (空 = 服务端 watchlist)。
    pub instruments: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            fixture_mode: std::env::var("GRPC_GATEWAY_TEST_FIXTURE").as_deref() == Ok("1"),
            shadow_events: std::env::var("GRPC_EVENTS_SHADOW").as_deref() == Ok("1"),
            port: std::env::var("GRPC_MARKET_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(18082),
            instruments: std::env::var("STOCK_LIST")
                .map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// 启动 gRPC 服务端。返回实际绑定地址 (port=0 时随机) 与 serve task。
pub async fn start(
    config: ServerConfig,
) -> anyhow::Result<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    log::info!(
        "[grpc_server] 监听 {bound} (fixture_mode={}, shadow_events={})",
        config.fixture_mode,
        config.shadow_events
    );

    let state = std::sync::Arc::new(ServerState {
        generation: format!("dev-{}", std::process::id()),
        sequence: AtomicU64::new(0),
        shadow_events: config.shadow_events,
    });

    let handle = tokio::spawn(async move {
        let health_svc = HealthService { state: state.clone() };
        let data_svc = handlers::DataService::new(state.clone(), config.fixture_mode);
        tonic::transport::Server::builder()
            .add_service(SystemServiceServer::new(health_svc))
            .add_service(handlers::market_data_service_server::MarketDataServiceServer::new(data_svc))
            // 注: MarketEventService 与 TdxAgentService 未注册 → tonic 对未注册服务
            // 返回 UNIMPLEMENTED (合同 §2 不做项)。MarketEventService 在 Task 11 注册。
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    Ok((bound, handle))
}

struct HealthService {
    state: std::sync::Arc<ServerState>,
}

#[tonic::async_trait]
impl SystemService for HealthService {
    async fn get_health(&self, _req: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            request_id: "health".to_string(),
            live: true,
            ready: true,
            state: "RUNNING".to_string(),
        }))
    }

    async fn get_capabilities(&self, _req: Request<CapabilitiesRequest>) -> Result<Response<CapabilitiesResponse>, Status> {
        let capabilities = implemented_operations()
            .into_iter()
            .map(|op| Capability {
                operation: op as i32,
                repository_admission: AdmissionState::Admitted as i32,
                runtime_available: true,
                provider: "tdx-dev".to_string(),
                exact_scope: "watchlist + explicit instruments".to_string(),
                blocker: String::new(),
            })
            .collect();
        Ok(Response::new(CapabilitiesResponse {
            request_id: "capabilities".to_string(),
            capabilities,
        }))
    }
}

// ListenerStatus 占位 (Task 11 实现): 当前只编译, 不注册方法。
#[allow(dead_code)]
pub(crate) fn listener_status_placeholder(
    _req: ListenerStatusRequest,
) -> Result<ListenerStatusResponse, Status> {
    Err(Status::unimplemented("Task 11 实现"))
}
```

- [x] **Step 2: 写 handlers.rs + delegate.rs + fixture.rs**

`src/grpc_server/handlers.rs`：

```rust
//! MarketDataService handler: 校验请求 → fixture 或 data_gateway 委托 → QueryResponse。
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::MarketDataService, AdmissionState, CanonicalPayload,
    Operation, QueryRequest, QueryResponse,
};
use crate::grpc_contract::schema::schema_for;
use crate::grpc_server::{delegate, fixture, ServerState};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub use crate::grpc_client::pb::magic::market::v1::market_data_service_server;

pub struct DataService {
    state: Arc<ServerState>,
    fixture_mode: bool,
}

impl DataService {
    pub fn new(state: Arc<ServerState>, fixture_mode: bool) -> Self {
        Self { state, fixture_mode }
    }

    /// 统一查询入口: 校验 → 取数 → 包装 QueryResponse。
    async fn serve_query(&self, op: Operation, req: QueryRequest) -> Result<Response<QueryResponse>, Status> {
        let payload = req
            .payload
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("QueryRequest 缺 payload"))?;
        let request_schema = payload.schema.clone();
        let request_version = payload.schema_version;

        // 合同 §5: 未知 schema/version 必须拒绝。
        let frozen = schema_for(op).ok_or_else(|| {
            Status::unimplemented(format!(
                "{} 未实现",
                crate::grpc_contract::ops::method_name(op)
            ))
        })?;
        if frozen.schema_name != request_schema {
            return Err(Status::invalid_argument(format!(
                "schema 不匹配: op 期望 {} 实际 {request_schema}",
                frozen.schema_name
            )));
        }
        if frozen.schema_version != request_version {
            return Err(Status::invalid_argument(format!(
                "schema 版本不支持: {} v{request_version} (冻结 v{})",
                frozen.schema_name, frozen.schema_version
            )));
        }

        // fixture 模式 (离线确定性测试) 优先。
        if self.fixture_mode {
            if let Some(resp) = fixture::fixture_response(op, &request_schema, request_version) {
                return Ok(Response::new(resp));
            }
            return Err(Status::unimplemented(format!(
                "{} 无 fixture",
                frozen.schema_name
            )));
        }

        // 真实路径: 委托 data_gateway (spawn_blocking 包同步调用)。
        let result = tokio::task::spawn_blocking({
            let op = op;
            let request_schema = request_schema.clone();
            move || delegate::fetch(op, &request_schema)
        })
        .await
        .map_err(|e| Status::internal(format!("gateway task 失败: {e}")))?
        .map_err(|e| Status::internal(e.to_string()))?;

        let request_id = req
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(Response::new(QueryResponse {
            request_id,
            operation: op as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: format!("{}-{}", frozen.schema_name, crate::grpc_client::envelope::new_request_id()),
            complete: true,
            observed_at: chrono::Local::now().to_rfc3339(),
            source_at: result.source_at,
            records: vec![CanonicalPayload {
                schema: frozen.schema_name.to_string(),
                schema_version: frozen.schema_version,
                content_type: "application/json; charset=utf-8".to_string(),
                data: result.data,
            }],
        }))
    }
}

// 54 个 RPC 的统一实现 (全部委托 serve_query; 未实现 op 返回 UNIMPLEMENTED)。
macro_rules! impl_unary_op {
    ($method:ident, $op:expr) => {
        async fn $method(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
            self.serve_query($op, req.into_inner()).await
        }
    };
}

#[tonic::async_trait]
impl MarketDataService for DataService {
    impl_unary_op!(historical_bars, Operation::HistoricalBars);
    impl_unary_op!(minute_data, Operation::MinuteData);
    impl_unary_op!(realtime_quotes, Operation::RealtimeQuotes);
    impl_unary_op!(money_flows, Operation::MoneyFlows);
    impl_unary_op!(order_books, Operation::OrderBooks);
    impl_unary_op!(auctions, Operation::Auctions);
    impl_unary_op!(trades, Operation::Trades);
    impl_unary_op!(security_metadata, Operation::SecurityMetadata);
    impl_unary_op!(global_indices, Operation::GlobalIndices);
    impl_unary_op!(foreign_exchange, Operation::ForeignExchange);
    impl_unary_op!(economic_calendar, Operation::EconomicCalendar);
    impl_unary_op!(futures_delivery, Operation::FuturesDelivery);
    impl_unary_op!(reference_rates, Operation::ReferenceRates);
    impl_unary_op!(official_fx_fixings, Operation::OfficialFxFixings);
    impl_unary_op!(economic_series, Operation::EconomicSeries);
    impl_unary_op!(company_filings, Operation::CompanyFilings);
    impl_unary_op!(global_news, Operation::GlobalNews);
    impl_unary_op!(announcements, Operation::Announcements);
    impl_unary_op!(market_announcements, Operation::MarketAnnouncements);
    impl_unary_op!(investor_questions, Operation::InvestorQuestions);
    impl_unary_op!(policy_documents, Operation::PolicyDocuments);
    impl_unary_op!(security_profiles, Operation::SecurityProfiles);
    impl_unary_op!(financial_statements, Operation::FinancialStatements);
    impl_unary_op!(market_statistics, Operation::MarketStatistics);
    impl_unary_op!(technical_bars, Operation::TechnicalBars);
    impl_unary_op!(corporate_actions, Operation::CorporateActions);
    impl_unary_op!(board_directory, Operation::BoardDirectory);
    impl_unary_op!(board_constituents, Operation::BoardConstituents);
    impl_unary_op!(board_memberships, Operation::BoardMemberships);
    impl_unary_op!(research_reports, Operation::ResearchReports);
    impl_unary_op!(research_documents, Operation::ResearchDocuments);
    impl_unary_op!(consensus, Operation::Consensus);
    impl_unary_op!(target_prices, Operation::TargetPrices);
    impl_unary_op!(semantic_search, Operation::SemanticSearch);
    impl_unary_op!(fund_flow_series, Operation::FundFlowSeries);
    impl_unary_op!(board_flows, Operation::BoardFlows);
    impl_unary_op!(margin_data, Operation::MarginData);
    impl_unary_op!(block_trades, Operation::BlockTrades);
    impl_unary_op!(holder_counts, Operation::HolderCounts);
    impl_unary_op!(lockup_events, Operation::LockupEvents);
    impl_unary_op!(dividend_plans, Operation::DividendPlans);
    impl_unary_op!(post_close_flows, Operation::PostCloseFlows);
    impl_unary_op!(northbound_daily, Operation::NorthboundDaily);
    impl_unary_op!(limit_pools, Operation::LimitPools);
    impl_unary_op!(strong_stock_reasons, Operation::StrongStockReasons);
    impl_unary_op!(dragon_tiger, Operation::DragonTiger);
    impl_unary_op!(market_dragon_tiger, Operation::MarketDragonTiger);
    impl_unary_op!(dragon_tiger_discovery, Operation::DragonTigerDiscovery);
    impl_unary_op!(market_rankings, Operation::MarketRankings);
    impl_unary_op!(market_breadth, Operation::MarketBreadth);
    impl_unary_op!(popularity, Operation::Popularity);
    impl_unary_op!(concept_hits, Operation::ConceptHits);
    impl_unary_op!(option_data, Operation::OptionData);
    impl_unary_op!(provider_top_n_rankings, Operation::ProviderTopNRankings);
}
```

`src/grpc_server/delegate.rs`（真实路径委托，Task 9/10 逐 op 填充；Task 8 先 6 个代表 op + 其余返回显式错误）：

```rust
//! data_gateway 委托层 (方案 A): 服务端进程内调用 data_gateway 取真实数据,
//! 序列化为 canonical JSON。fixture_mode 下不经过这里。
//! 每个 op 一个 fetch_xxx(schema: &str) -> Result<Fetched, String>。
use crate::grpc_client::pb::magic::market::v1::Operation;

pub struct Fetched {
    pub data: Vec<u8>,
    pub source_at: String,
}

fn not_yet(op: Operation) -> Result<Fetched, String> {
    Err(format!(
        "{}: delegate 尚未实现 (Task 9/10 补全)",
        crate::grpc_contract::ops::method_name(op)
    ))
}

pub fn fetch(op: Operation, schema: &str) -> Result<Fetched, String> {
    match op {
        Operation::RealtimeQuotes => fetch_realtime_quotes(),
        Operation::HistoricalBars => fetch_historical_bars(),
        Operation::MinuteData => fetch_minute_data(),
        Operation::Announcements => fetch_announcements(),
        Operation::GlobalNews => fetch_global_news(),
        Operation::SecurityMetadata => fetch_security_metadata(),
        _ => not_yet(op),
    }
}

/// 真实路径: 统一实时行情 Gateway。字段名以编译/实测为准微调。
pub fn fetch_realtime_quotes() -> Result<Fetched, String> {
    let codes = std::env::var("STOCK_LIST")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let batch = crate::data_gateway::MarketDataGateway::new()
        .realtime_quotes(&codes)
        .map_err(|e| format!("统一实时行情 Gateway 不可用: {e}"))?;
    let records: Vec<serde_json::Value> = batch
        .stocks
        .iter()
        .map(|s| {
            serde_json::json!({
                "code": s.code,
                "name": s.name,
                "price": s.price,
                "change_pct": s.change_pct,
                "volume": s.volume,
                "amount": s.amount,
            })
        })
        .collect();
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        source_at: chrono::Local::now().to_rfc3339(),
    })
}

// Task 9/10: 其余 5 个代表 op + 全部生产 op 的 fetch_xxx 逐个落地;
// 每个 op 落地时先 grep data_gateway 对应 Gateway 的返回类型字段名再写 JSON 映射。
fn fetch_historical_bars() -> Result<Fetched, String> { not_yet(Operation::HistoricalBars) }
fn fetch_minute_data() -> Result<Fetched, String> { not_yet(Operation::MinuteData) }
fn fetch_announcements() -> Result<Fetched, String> { not_yet(Operation::Announcements) }
fn fetch_global_news() -> Result<Fetched, String> { not_yet(Operation::GlobalNews) }
fn fetch_security_metadata() -> Result<Fetched, String> { not_yet(Operation::SecurityMetadata) }
```

`src/grpc_server/fixture.rs`：

```rust
//! fixture 数据 (离线确定性测试): GRPC_GATEWAY_TEST_FIXTURE=1 时 handler 返回这些数据。
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, CanonicalPayload, Operation, QueryResponse,
};

pub fn fixture_response(op: Operation, schema: &str, version: u32) -> Option<QueryResponse> {
    let payload = |data: &[u8]| CanonicalPayload {
        schema: schema.to_string(),
        schema_version: version,
        content_type: "application/json; charset=utf-8".to_string(),
        data: data.to_vec(),
    };
    let resp = |request_id: &str, records: Vec<CanonicalPayload>| QueryResponse {
        request_id: request_id.to_string(),
        operation: op as i32,
        admission: AdmissionState::Admitted as i32,
        selected_provider: "fixture".to_string(),
        batch_id: "fixture-b1".to_string(),
        complete: true,
        observed_at: "2026-08-13T10:00:00+08:00".to_string(),
        source_at: "2026-08-13T10:00:00+08:00".to_string(),
        records,
    };
    match op {
        Operation::RealtimeQuotes => Some(resp(
            "fixture-rq",
            vec![payload(
                br#"[{"code":"600519","name":"贵州茅台","price":1500.0,"change_pct":2.34,"volume":12345,"amount":1.85e9}]"#,
            )],
        )),
        Operation::HistoricalBars => Some(resp(
            "fixture-hb",
            vec![payload(
                br#"[{"code":"600519","date":"2026-08-13","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":12345}]"#,
            )],
        )),
        Operation::MinuteData => Some(resp(
            "fixture-md",
            vec![payload(
                br#"[{"code":"600519","time":"09:35","open":1490.0,"high":1505.0,"low":1488.0,"close":1500.0,"volume":1200}]"#,
            )],
        )),
        Operation::Announcements => Some(resp(
            "fixture-ann",
            vec![payload(
                br#"[{"code":"600519","title":"贵州茅台:关于2026年中期分红的公告","published_at":"2026-08-13T09:00:00+08:00","url":"https://example.com/a1"}]"#,
            )],
        )),
        Operation::GlobalNews => Some(resp(
            "fixture-news",
            vec![payload(
                br#"[{"title":"央行开展逆回购操作","source":"fixture-news","published_at":"2026-08-13T08:30:00+08:00","url":"https://example.com/n1"}]"#,
            )],
        )),
        Operation::SecurityMetadata => Some(resp(
            "fixture-sec",
            vec![payload(
                br#"[{"code":"600519","name":"贵州茅台","market":"SH","industry":"白酒","list_date":"2001-08-27"}]"#,
            )],
        )),
        _ => None,
    }
}
```

- [x] **Step 3: 写薄二进制**

`src/bin/grpc_market_server.rs`：

```rust
//! gRPC mock 服务端 (合同 grpc/grpc-external-api.md, 方案 A 委托 data_gateway)。
//! 默认 127.0.0.1:18082; GRPC_MARKET_PORT / GRPC_GATEWAY_TEST_FIXTURE / GRPC_EVENTS_SHADOW 可配。
//! 只读数据服务 + TDX 异动事件订阅。无账户/持仓/委托写接口。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = stock_analysis::grpc_server::ServerConfig::default();
    let (addr, handle) = stock_analysis::grpc_server::start(config).await?;
    log::info!("[grpc_market_server] 就绪: {addr} (Ctrl-C 退出)");
    tokio::select! {
        r = handle => r??,
        _ = tokio::signal::ctrl_c() => log::info!("[grpc_market_server] 收到 Ctrl-C, 退出"),
    }
    Ok(())
}
```

`src/lib.rs` 追加 `pub mod grpc_server;`。

- [x] **Step 4: 写集成测试**

`tests/grpc_channel_e2e.rs`：

```rust
//! 集成测试: 真起 grpc_server (fixture 模式, 随机端口) → GrpcMarketClient 调用。
//! 离线确定性, 不连真实网络。
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::pb::magic::market::v1::Operation;
use stock_analysis::grpc_server::{start, ServerConfig};

#[tokio::test(flavor = "multi_thread")]
async fn health_and_capabilities() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let addr = format!("http://{addr}");
    let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
    let health = client.get_health().await.unwrap();
    assert!(health.live && health.ready);
    let caps = client.get_capabilities().await.unwrap();
    assert_eq!(caps.len(), 24, "24 个生产 op 全部在 capability 表");
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn six_representative_ops_fixture_roundtrip() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}")).await.unwrap();

    let cases: Vec<(Operation, &str, &str)> = vec![
        (Operation::RealtimeQuotes, "market.realtime_quotes", "600519"),
        (Operation::HistoricalBars, "market.historical_bars", "600519"),
        (Operation::MinuteData, "market.minute_data", "600519"),
        (Operation::Announcements, "news.announcements", "600519"),
        (Operation::GlobalNews, "news.global_news", "央行"),
        (Operation::SecurityMetadata, "market.security_metadata", "600519"),
    ];
    for (op, schema, probe) in cases {
        let result = client
            .query(op, serde_json::json!({}))
            .unwrap_or_else(|e| panic!("{schema} 查询失败: {e}"));
        assert!(result.complete, "{schema} complete=true");
        assert_eq!(result.records.len(), 1, "{schema} 1 条 fixture 记录");
        assert_eq!(result.records[0].schema, schema);
        let parsed: serde_json::Value = serde_json::from_slice(&result.records[0].data).unwrap();
        assert!(parsed[0].to_string().contains(probe), "{schema} 内容含 {probe}");
    }
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_schema_rejected() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}")).await.unwrap();
    // OptionData 未实现 → 客户端拦截。
    let err = client
        .query(Operation::OptionData, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        stock_analysis::grpc_client::errors::GrpcError::Unimplemented
    ));
    handle.abort();
}
```

- [x] **Step 5: 跑测试**

Run: `cargo test --test grpc_channel_e2e 2>&1 | tail -8`
Expected: PASS（3 passed）。`cargo build --lib` 与 `cargo build --bin grpc_market_server` exit 0。

- [x] **Step 6: 手工真实模式冒烟（可选，需网络）**

Run: `cargo run --bin grpc_market_server 2>&1 | head -3`（另开终端）
Expected: 启动日志 `[grpc_server] 监听 127.0.0.1:18082`。真实 provider 路径在 Task 9/10 后验证。

- [x] **Step 7: Commit**

```bash
git add src/grpc_server/ src/bin/grpc_market_server.rs src/lib.rs tests/grpc_channel_e2e.rs
git commit -m "feat(grpc): P1 grpc_market_server 骨架 (SystemService + 6 代表 op fixture 模式 + 集成测试)"
```

---

### Task 9: delegate 层补全 — 核心 12 op 委托 data_gateway（真实路径）

**Files:**
- Modify: `src/grpc_server/delegate.rs`（新增 12 个 fetch_xxx）

**Interfaces:**
- Consumes: `data_gateway` 各 Gateway（现有库代码，只读调用）
- Produces: `pub fn fetch(op, schema) -> Result<Fetched, String>` 覆盖 18 个 op（6 代表 + 12 核心）

- [x] **Step 1: 核对 data_gateway 入口与字段**

Run: `grep -n "pub async fn\|pub fn" src/data_gateway/market_capabilities.rs | head -20` 及 `grep -rn "pub struct TopStock" -A 15 src/`
Expected: 记录 12 个 op 对应的 Gateway 入口函数名与返回结构体字段（写进 delegate.rs 的注释，禁止凭记忆写字段）。

12 个核心 op 的委托目标（探索结果锚点，落地时逐一核对签名）：
- MinuteData → `MarketCapabilitiesGateway::minute_data` (src/data_gateway/market_capabilities.rs:161)
- OrderBooks → `MarketCapabilitiesGateway::order_books` (:310)
- MoneyFlows → `MarketCapabilitiesGateway::money_flows`
- SecurityMetadata → `MarketCapabilitiesGateway::security_metadata`
- GlobalIndices → `GlobalMarketGateway`/`TencentClient` (src/data_gateway/global_market.rs:145, index.rs:59)
- Announcements → Cninfo 公告 (src/data_gateway/event_calendar.rs:100)
- GlobalNews → `GlobalNewsGateway` (src/data_gateway/global_news.rs:142)
- EconomicCalendar → (src/data_gateway/economic_calendar.rs:110)
- FuturesDelivery → (src/data_gateway/futures_delivery.rs:86)
- DragonTiger → `DragonTigerGateway` (src/data_gateway/dragon_tiger.rs:123)
- BlockTrades → (src/data_gateway/block_trade.rs:72)
- Consensus → (src/data_gateway/consensus.rs:77)

- [x] **Step 2: 逐个实现 fetch_xxx**

`delegate.rs` 的 `fetch()` match 扩到 18 个 op。每个 fetch 模式（以 minute_data 为例）：

```rust
fn fetch_minute_data() -> Result<Fetched, String> {
    // 先 grep MarketCapabilitiesGateway::minute_data 的真实签名/返回类型, 再写映射。
    let gateway = crate::data_gateway::MarketCapabilitiesGateway::new();
    let codes = watchlist_codes();
    let batch = gateway
        .minute_data(&codes)
        .map_err(|e| format!("分钟线 Gateway 不可用: {e}"))?;
    let records: Vec<serde_json::Value> = batch
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        source_at: String::new(), // evidence 无可信源时间 → 空 (合同 §6: 不填充)
    })
}

fn watchlist_codes() -> Vec<String> {
    std::env::var("STOCK_LIST")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
```

规则：每个 op 的 JSON 字段名 = 该 Gateway 返回结构体的 serde 序列化字段（用 `serde_json::to_value` 直出，保持 schema 稳定）；`source_at` 取 evidence 的可信源时间，缺则空。

- [x] **Step 3: 编译 + 现有测试回归**

Run: `cargo build --lib 2>&1 | tail -3` 和 `cargo test --test grpc_channel_e2e 2>&1 | tail -3`
Expected: 都通过（fixture 集成测试不受 delegate 改动影响）。

- [x] **Step 4: Commit**

```bash
git add src/grpc_server/delegate.rs
git commit -m "feat(grpc): P2 delegate 委托层 核心 12 op 走真实 data_gateway (source_at 原样)"
```

---

### Task 10: delegate 层补全 — 其余 12 op + capability 表核对

**Files:**
- Modify: `src/grpc_server/delegate.rs`

**Interfaces:**
- Consumes: 同 Task 9
- Produces: `fetch()` 覆盖全部 24 个生产 op

- [x] **Step 1: 核对剩余 12 op 的 Gateway 锚点**

- BoardDirectory / BoardConstituents → `BoardDataGateway` (src/data_gateway/board_runtime.rs:392/:452)
- BoardFlows → Eastmoney 板块资金流 (src/data_gateway/capital.rs:335)
- LimitPools / StrongStockReasons → `ChainIntelligenceGateway` (src/data_gateway/chain_intelligence.rs:855)
- MarketDragonTiger → (src/data_gateway/dragon_tiger.rs)
- MarketRankings / ConceptHits → (src/data_gateway/board_ranking.rs — reqwest 直连, 或 chain_intelligence)
- ResearchReports → (src/data_gateway/research.rs:85)
- NorthboundDaily → (src/data_gateway/capital.rs)
- HistoricalBars → `HistoricalBarsGateway::{daily_bars, fifteen_min_bars}` (src/data_gateway/historical_bars.rs:181/:230)

每个入口 grep 签名后实现 fetch_xxx，规则同 Task 9 Step 2。

- [x] **Step 2: 实现 + 编译 + 测试**

Run: `cargo build --lib`、`cargo test --test grpc_channel_e2e`、`cargo test --lib grpc_contract::`
Expected: 全绿。

- [x] **Step 3: 真实模式手工冒烟（24 op 逐个 probe）**

最简做法：`cargo run --bin grpc_market_server` 后用一个临时集成测试（`#[ignore]`）或临时 test 二进制逐个 op 查询断言 `admission=ADMITTED, complete=true`。不修改 monitor 与生产路径。
Expected: 真实模式下 24 op 返回 `admission=ADMITTED, complete=true`（网络可用时）。

- [x] **Step 4: Commit**

```bash
git add src/grpc_server/delegate.rs
git commit -m "feat(grpc): P2 delegate 委托层 24 个生产 op 全量覆盖"
```

---

### Task 11: 事件生成器 + MarketEventService（Subscribe/Replay/GetListenerStatus）

**Files:**
- Create: `src/grpc_server/events.rs`（diff 检测器 + EventHub + MarketEventService impl）
- Modify: `src/grpc_server/mod.rs`（start() 返回三元素 tuple 带 hub）
- Modify: `tests/grpc_channel_e2e.rs`（订阅流集成测试）

**Interfaces:**
- Consumes: Task 8 `ServerConfig`/`ServerState`；Task 1 生成代码（Subscribe/Replay/GetListenerStatus 消息）
- Produces:
  - `pub enum EventKind { Price, Volume, Amount, Status, Reset }` + `pub fn as_str(&self) -> &'static str`
  - `pub struct Quote { pub code: String, pub name: String, pub price: f64, pub prev_close: f64, pub volume: u64, pub amount: f64 }` + `pub fn change_pct(&self) -> f64`
  - `pub struct DetectedEvent { pub kind: EventKind, pub code: String, pub name: String, pub price: f64, pub prev_close: f64, pub change_pct: f64, pub volume: u64, pub amount: f64, pub reason: String }`
  - `pub(crate) fn diff_snapshots(prev: &[Quote], next: &[Quote], threshold_pct: f64, volume_x: f64) -> Vec<DetectedEvent>` — 纯函数
  - `pub struct EventHub { generation: String, sequence: AtomicU64, tx: broadcast::Sender<MarketEventEnvelope>, ring: Mutex<VecDeque<MarketEventEnvelope>>, shadow_events: bool }` + `pub fn new(generation: String, shadow_events: bool) -> Self` + `pub fn push_event(&self, event: &DetectedEvent)` + `pub fn latest_cursor(&self) -> EventCursor` + `pub fn replay_after(&self, cursor: Option<EventCursor>) -> Result<Vec<MarketEventEnvelope>, Status>`
  - `pub struct EventService { pub hub: Arc<EventHub> }` + `impl EventService::new(state: Arc<ServerState>, fixture_mode: bool) -> Self`（fixture 模式不启动轮询，只接受注入）
  - `pub fn poll_interval_ms() -> u64`（`EVENT_POLL_INTERVAL_MS`，默认 3000）/ `pub fn thresholds() -> (f64, f64)`（`EVENT_PRICE_THRESHOLD_PCT` 默认 0.5 pp、`EVENT_VOLUME_THRESHOLD_X` 默认 1.5x，spec §4.5 可配置）
  - `EventKind::Reset` 为合同定义事件（§8）：服务重启/代次切换由 generation 变化表达（Replay FAILED_PRECONDITION），reset 事件的消费侧语义在 P3 listener 落地

- [x] **Step 1: diff 检测器（先测后码）**

`src/grpc_server/events.rs` 上半部分：

```rust
//! TDX 异动事件生成器 (合同 §8: price/volume/amount/status/reset 事件;
//! cursor generation+sequence 单调递增; UNADMITTED 影子事件必须显式隔离)。
use crate::grpc_client::pb::magic::market::v1::{
    market_event_service_server::MarketEventService, AdmissionState, CanonicalPayload,
    EventCursor, EventFilter, ListenerStatusRequest, ListenerStatusResponse,
    MarketEventEnvelope, ReplayRequest, SubscribeRequest,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    Price,
    Volume,
    Amount,
    Status,
    Reset,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Price => "price",
            EventKind::Volume => "volume",
            EventKind::Amount => "amount",
            EventKind::Status => "status",
            EventKind::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub prev_close: f64,
    pub volume: u64,
    pub amount: f64,
}

impl Quote {
    pub fn change_pct(&self) -> f64 {
        if self.prev_close <= 0.0 {
            0.0
        } else {
            (self.price - self.prev_close) / self.prev_close * 100.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedEvent {
    pub kind: EventKind,
    pub code: String,
    pub name: String,
    pub price: f64,
    pub prev_close: f64,
    pub change_pct: f64,
    pub volume: u64,
    pub amount: f64,
    pub reason: String,
}

/// 快照 diff: 涨跌幅变化 ≥ threshold_pct 百分点 → Price;
/// 成交量/成交额相对上一快照突增 ≥ volume_x 倍 → Volume/Amount;
/// 停牌 (volume=0) / 复牌 转换 → Status。纯函数, 离线单测。
pub(crate) fn diff_snapshots(
    prev: &[Quote],
    next: &[Quote],
    threshold_pct: f64,
    volume_x: f64,
) -> Vec<DetectedEvent> {
    let mut events = Vec::new();
    for q in next {
        let Some(p) = prev.iter().find(|p| p.code == q.code) else {
            // 新出现标的: 只作为初始快照, 不产生事件 (避免启动刷屏)。
            continue;
        };
        let change = (q.change_pct() - p.change_pct()).abs();
        if change >= threshold_pct {
            events.push(DetectedEvent {
                kind: EventKind::Price,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("涨跌幅变化 {change:.2}pp"),
            });
        }
        if p.volume > 0 && q.volume as f64 >= p.volume as f64 * volume_x {
            events.push(DetectedEvent {
                kind: EventKind::Volume,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("成交量突增 {:.1}x", q.volume as f64 / p.volume as f64),
            });
        }
        if p.amount > 0.0 && q.amount >= p.amount * volume_x {
            events.push(DetectedEvent {
                kind: EventKind::Amount,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("成交额突增 {:.1}x", q.amount / p.amount),
            });
        }
        let was_halted = p.volume == 0;
        let now_halted = q.volume == 0;
        if was_halted != now_halted {
            events.push(DetectedEvent {
                kind: EventKind::Status,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: if now_halted { "停牌".to_string() } else { "复牌".to_string() },
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: &str, price: f64, prev_close: f64, volume: u64, amount: f64) -> Quote {
        Quote {
            code: code.to_string(),
            name: format!("n-{code}"),
            price,
            prev_close,
            volume,
            amount,
        }
    }

    #[test]
    fn detects_price_movement() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1520.0, 1500.0, 100, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.iter().any(|e| e.kind == EventKind::Price));
    }

    #[test]
    fn ignores_small_movement() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1501.0, 1500.0, 100, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.is_empty());
    }

    #[test]
    fn detects_volume_spike() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1500.0, 1500.0, 400, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.iter().any(|e| e.kind == EventKind::Volume));
    }

    #[test]
    fn detects_halt_and_resume() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let halted = vec![q("600519", 1500.0, 1500.0, 0, 0.0)];
        let resumed = vec![q("600519", 1500.0, 1500.0, 50, 5e7)];
        let e1 = diff_snapshots(&prev, &halted, 0.5, 1.5);
        assert!(e1.iter().any(|e| e.kind == EventKind::Status && e.reason == "停牌"));
        let e2 = diff_snapshots(&halted, &resumed, 0.5, 1.5);
        assert!(e2.iter().any(|e| e.kind == EventKind::Status && e.reason == "复牌"));
    }

    #[test]
    fn new_code_in_snapshot_does_not_spam() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![
            q("600519", 1500.0, 1500.0, 100, 1e8),
            q("000001", 10.0, 10.0, 1000, 1e6),
        ];
        assert!(diff_snapshots(&prev, &next, 0.5, 1.5).is_empty());
    }
}
```

- [x] **Step 2: 跑 diff 单测**

Run: `cargo test --lib grpc_server::events:: 2>&1 | tail -6`
Expected: PASS（5 passed）。

- [x] **Step 3: 写 EventHub + MarketEventService impl**

继续 `src/grpc_server/events.rs`：

```rust
pub struct EventHub {
    generation: String,
    sequence: AtomicU64,
    tx: broadcast::Sender<MarketEventEnvelope>,
    ring: Mutex<VecDeque<MarketEventEnvelope>>,
    shadow_events: bool,
}

const RING_CAPACITY: usize = 10_000;

impl EventHub {
    pub fn new(generation: String, shadow_events: bool) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            generation,
            sequence: AtomicU64::new(0),
            tx,
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            shadow_events,
        }
    }

    pub fn push_event(&self, event: &DetectedEvent) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let envelope = MarketEventEnvelope {
            protocol_version: 1,
            event_id: crate::grpc_client::envelope::new_request_id(),
            cursor: Some(EventCursor { generation: self.generation.clone(), sequence }),
            event_kind: event.kind.as_str().to_string(),
            provider: "tdx-dev".to_string(),
            instrument: event.code.clone(),
            observed_at: chrono::Local::now().to_rfc3339(),
            source_at: String::new(), // 轮询行情无可信源时间 → 空 (合同 §6 不填充)
            admission: if self.shadow_events {
                AdmissionState::Unadmitted as i32
            } else {
                AdmissionState::Admitted as i32
            },
            payload: Some(CanonicalPayload {
                schema: "market_event".to_string(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_string(),
                data: serde_json::to_vec(&serde_json::json!({
                    "code": event.code, "name": event.name, "price": event.price,
                    "prev_close": event.prev_close, "change_pct": event.change_pct,
                    "volume": event.volume, "amount": event.amount, "reason": event.reason,
                }))
                .unwrap_or_default(),
            }),
        };
        let mut ring = self.ring.lock().unwrap();
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(envelope.clone());
        drop(ring);
        let _ = self.tx.send(envelope);
    }

    pub fn latest_cursor(&self) -> EventCursor {
        let ring = self.ring.lock().unwrap();
        let seq = ring
            .back()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        EventCursor { generation: self.generation.clone(), sequence: seq }
    }

    /// Replay: 有界、同 generation、best-effort (合同 §8)。
    pub fn replay_after(&self, cursor: Option<EventCursor>) -> Result<Vec<MarketEventEnvelope>, Status> {
        let ring = self.ring.lock().unwrap();
        let Some(cursor) = cursor else {
            return Ok(ring.iter().cloned().collect());
        };
        if cursor.generation != self.generation {
            return Err(Status::failed_precondition("generation 不匹配, 连续性已重置"));
        }
        let latest = ring
            .back()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        if latest < cursor.sequence {
            // cursor 未来值 → 空重放 (可能服务端重启后 sequence 回退)。
            return Ok(vec![]);
        }
        let oldest = ring
            .front()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        if cursor.sequence < oldest {
            return Err(Status::out_of_range("cursor 早于重放窗口, 记录明确 gap"));
        }
        Ok(ring
            .iter()
            .filter(|e| {
                e.cursor
                    .as_ref()
                    .map(|c| c.sequence > cursor.sequence)
                    .unwrap_or(false)
            })
            .cloned()
            .collect())
    }
}

pub struct EventService {
    pub hub: Arc<EventHub>,
}

impl EventService {
    /// fixture 模式: hub 只接受外部注入 (集成测试调 push_event), 不启动轮询。
    pub fn new(state: Arc<ServerState>, _fixture_mode: bool) -> Self {
        Self {
            hub: Arc::new(EventHub::new(state.generation.clone(), state.shadow_events)),
        }
    }
}

fn envelope_matches(e: &MarketEventEnvelope, f: &EventFilter) -> bool {
    let ok_instrument = f.instruments.is_empty() || f.instruments.contains(&e.instrument);
    let ok_kind = f.event_kinds.is_empty() || f.event_kinds.contains(&e.event_kind);
    ok_instrument && ok_kind
}

#[tonic::async_trait]
impl MarketEventService for EventService {
    type SubscribeStream =
        tokio_stream::wrappers::ReceiverStream<Result<MarketEventEnvelope, Status>>;

    async fn subscribe(
        &self,
        req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let inner = req.into_inner();
        let filter = inner.filter.unwrap_or(EventFilter {
            instruments: vec![],
            event_kinds: vec![],
        });
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let hub = self.hub.clone();
        let mut live_rx = hub.tx.subscribe();
        let replay = hub.replay_after(inner.after)?;
        tokio::spawn(async move {
            for envelope in replay {
                if envelope_matches(&envelope, &filter) {
                    if tx.send(Ok(envelope)).await.is_err() {
                        return;
                    }
                }
            }
            while let Ok(envelope) = live_rx.recv().await {
                if envelope_matches(&envelope, &filter) {
                    if tx.send(Ok(envelope)).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn replay(
        &self,
        req: Request<ReplayRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let inner = req.into_inner();
        let envelopes = self.hub.replay_after(inner.after)?;
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            for envelope in envelopes {
                if tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn get_listener_status(
        &self,
        _req: Request<ListenerStatusRequest>,
    ) -> Result<Response<ListenerStatusResponse>, Status> {
        let cursor = self.hub.latest_cursor();
        Ok(Response::new(ListenerStatusResponse {
            request_id: "status".to_string(),
            state: "RUNNING".to_string(),
            terminal_generation: cursor.generation.clone(),
            latest: Some(cursor),
            capabilities: vec![],
        }))
    }
}

pub use crate::grpc_client::pb::magic::market::v1::market_event_service_server;

/// 轮询间隔 (EVENT_POLL_INTERVAL_MS, 默认 3000ms)。
/// v15.x: 默认值出声 — 调用方启动时必须打印实际生效值。
pub fn poll_interval_ms() -> u64 {
    std::env::var("EVENT_POLL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000)
}

/// 异动阈值 (EVENT_PRICE_THRESHOLD_PCT 百分点 / EVENT_VOLUME_THRESHOLD_X 倍数,
/// 默认 0.5 / 1.5)。v15.x: 调用方启动时必须打印实际生效值。
pub fn thresholds() -> (f64, f64) {
    let pct = std::env::var("EVENT_PRICE_THRESHOLD_PCT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5);
    let x = std::env::var("EVENT_VOLUME_THRESHOLD_X")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.5);
    (pct, x)
}
```

- [x] **Step 4: start() 返回 hub + 轮询循环注入**

`src/grpc_server/mod.rs` 改 `start()` 签名：`-> anyhow::Result<(SocketAddr, JoinHandle<Result<(), tonic::transport::Error>>, Arc<events::EventHub>)>`，构造 `events::EventService::new(state.clone(), config.fixture_mode)` 并 `add_service`。同步修改 Task 8 集成测试的元组解构（`let (addr, handle, _hub) = start(...)`）。

薄二进制 `src/bin/grpc_market_server.rs` 追加轮询循环（真实 provider 模式，fixture 模式由集成测试注入）：

```rust
// 在 start() 之后:
let (addr, handle, hub) = stock_analysis::grpc_server::start(config).await?;
let poll = stock_analysis::grpc_server::events::poll_interval_ms();
let (price_t, vol_t) = stock_analysis::grpc_server::events::thresholds();
log::info!(
    "[grpc_market_server] 事件轮询间隔 {poll}ms, 阈值 {price_t:.2}pp/{vol_t:.2}x"
); // 默认值出声 (v15.x)

let hub_for_poll = hub.clone();
tokio::spawn(async move {
    let mut prev: Vec<stock_analysis::grpc_server::events::Quote> = Vec::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(poll)).await;
        let codes: Vec<String> = std::env::var("STOCK_LIST")
            .map(|s| {
                s.split(',')
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let Ok(batch) = stock_analysis::data_gateway::MarketDataGateway::new()
            .realtime_quotes(&codes)
        else {
            continue; // 拉取失败跳过本周期, 保留上一快照
        };
        // 字段名以 TopStock 实际结构为准 (实现时 grep 确认), 与 delegate.rs 同步。
        let next: Vec<stock_analysis::grpc_server::events::Quote> = batch
            .stocks
            .iter()
            .map(|s| stock_analysis::grpc_server::events::Quote {
                code: s.code.clone(),
                name: s.name.clone(),
                price: s.price,
                prev_close: s.prev_close,
                volume: s.volume,
                amount: s.amount,
            })
            .collect();
        let events = stock_analysis::grpc_server::events::diff_snapshots(&prev, &next, price_t, vol_t);
        for e in events {
            hub_for_poll.push_event(&e);
        }
        prev = next;
    }
});
```

- [x] **Step 5: 订阅流集成测试**

`tests/grpc_channel_e2e.rs` 追加：

```rust
use stock_analysis::grpc_client::pb::magic::market::v1::{EventCursor, EventFilter};
use stock_analysis::grpc_server::events::{DetectedEvent, EventHub, EventKind};

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_injected_events_with_monotonic_cursor() {
    let (addr, handle, hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}")).await.unwrap();
    let mut stream = client
        .subscribe(
            EventFilter { instruments: vec![], event_kinds: vec![] },
            None,
        )
        .await
        .unwrap();

    let d = DetectedEvent {
        kind: EventKind::Price,
        code: "600519".into(),
        name: "贵州茅台".into(),
        price: 1520.0,
        prev_close: 1500.0,
        change_pct: 1.33,
        volume: 100,
        amount: 1e8,
        reason: "涨跌幅变化".into(),
    };
    hub.push_event(&d);

    use futures::StreamExt;
    let envelope = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.next(),
    )
    .await
    .expect("5s 内收到事件")
    .expect("流未结束")
    .expect("事件无错误");
    assert_eq!(envelope.instrument, "600519");
    assert_eq!(envelope.event_kind, "price");
    let cursor = envelope.cursor.unwrap();
    assert_eq!(cursor.sequence, 1);
    assert_eq!(cursor.generation, hub.latest_cursor().generation);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_returns_bounded_events_same_generation() {
    let hub = EventHub::new("g1".to_string(), false);
    let d = DetectedEvent {
        kind: EventKind::Price,
        code: "600519".into(),
        name: "贵州茅台".into(),
        price: 1520.0,
        prev_close: 1500.0,
        change_pct: 1.33,
        volume: 100,
        amount: 1e8,
        reason: "涨跌幅变化".into(),
    };
    hub.push_event(&d);
    let q = hub
        .replay_after(Some(EventCursor { generation: "g1".into(), sequence: 0 }))
        .unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].instrument, "600519");
    // generation 不匹配 → FAILED_PRECONDITION
    let err = hub
        .replay_after(Some(EventCursor { generation: "g2".into(), sequence: 0 }))
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}
```

（`futures = "0.3"` 已在 Cargo.toml [dependencies]，集成测试可直接使用，无需新增 dev-dependency。）

- [x] **Step 6: 跑测试**

Run: `cargo test --test grpc_channel_e2e 2>&1 | tail -8` 和 `cargo test --lib grpc_server:: 2>&1 | tail -5`
Expected: 全绿。

- [x] **Step 7: Commit**

```bash
git add src/grpc_server/ src/bin/grpc_market_server.rs tests/grpc_channel_e2e.rs Cargo.toml Cargo.lock
git commit -m "feat(grpc): P2 事件生成器 + Subscribe/Replay/GetListenerStatus (diff 检测单测 + 订阅流集成测试)"
```

---

## 本计划完成后的状态

- gRPC 数据通道（P0-P2）交付：客户端 24 op 可查、服务端真实 provider 委托、事件流可用。
- **现有程序零改动**：monitor 二进制/14 处 push_governor_v3/289 单测未触碰；`Cargo.toml` 仅加 tonic/prost/tokio-stream/tonic-build 4 个依赖（thiserror/chrono/log/env_logger/futures/serde_json 均已在册）。
- 未做（下个计划）：P3 监听器 + 飞书推送（grpc_event_listener + push_forward + 新 PushKind）；P4 data_gateway 开关迁移；P5 文档交接。
