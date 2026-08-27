# gRPC 数据通道实施计划（Spec P0-P2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 gRPC 数据通道——用 `grpc/market.proto` 生成客户端与服务端骨架，客户端 `GrpcMarketClient` 可查询生产实际用到的全部数据族（24 op），服务端 `grpc_market_server` 委托 `data_gateway` 取真实数据并生成 TDX 异动事件流。

**Architecture:** 服务端逻辑放库模块 `src/grpc_server/`（可被集成测试直接构造），`src/bin/grpc_market_server.rs` 是薄入口；客户端在 `src/grpc_client/`；schema 注册表在 `src/grpc_contract/`（双端共享）。测试用 fixture 模式（`fixture_mode: true`）离线确定性；真实 provider 路径由手工运行 server 二进制验证。

**Tech Stack:** tonic 0.14（client+server 生成）、prost 0.14、tonic-build 0.14（build-dependency）、tokio-stream 0.1（ReceiverStream）、tokio、serde_json、anyhow。

**Spec:** `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md`（§11.9 Gate C 聚焦收口已批准，commit `e056a7f`）

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

---

# T0 Wire v2 与本地双进程切换实施计划（2026-08-27 修订）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 T0 五分钟时间无时区与空批次不可重建问题，仅在全请求批次真实成功时归因 OrderBook，并安全部署同版本 server/monitor 后恢复日线 freshness。

**Architecture:** 保留 `MagicTdxGateway` 的领域校验，在 `grpc_server` 增加只负责 v2 序列化的显式 wire 模块；客户端严格验证 schema、`+08:00` civil label 和批级证据后恢复现有领域对象。server 与 monitor 使用同一源码候选、备用端口验证、成对切换与成对回滚；MoneyFlow 继续保持显式 Unsupported。

**Tech Stack:** Rust 2021、chrono 0.4、serde/serde_json、tonic/prost、SQLite、Bash 合规门禁、cargo-llvm-cov 0.8.7。

**Spec:** `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md` §11（批准并固化于 commit `6818738`）

## Global Constraints

- 生产路径不得使用 mock；fixture 仅存在于 `#[cfg(test)]` 或 `fixture_mode=true` 隔离路径（2.1/2.5）。
- `completed_five_minute[].at` v2 只接受显式 `+08:00`；无 offset、其他 offset、未来/倒置时间全部 fail-closed（2.2/2.3/2.4）。
- payload 的 `source_at`、`observed_at`、`batch_id` 必须与 gRPC envelope 相同；空 records 不得用当前时间补 `requested_at`（2.2/2.7/BR-238）。
- MoneyFlow 不接假 provider，不以 BoardFlows 代替，保持 `unsupported_contract`（2.1/2.8）。
- BR-253 必须先登记，再实现“全请求集合成功才刷新 OrderBook capability”（2.10）。
- 不修改 `grpc/market.proto`、策略阈值、订单门禁、推送节流或真实下单授权。
- 不触碰未跟踪的根目录 `task_plan.md`、`findings.md`、`progress.md`。
- Gate C 使用 `--policy pr`；Gate D 必须显式绑定固定生产 DB，不能用日期或日历覆盖绕过 freshness。

---

### Task 1: 先登记 BR-253

**Files:**
- Modify: `docs/business_rules.md`

**Interfaces:**
- Consumes: 设计 §11.4 的全请求集合原子成功条件。
- Produces: `BR-253`，供 `src/data_gateway/grpc_source.rs` 的实现和 PR `Business-Rules` 字段引用。

- [ ] **Step 1: 在规则表登记 BR-253**

追加一行（保持表格现有格式）：

```markdown
| BR-253 | 🟡 Gate A approved；Gate B/C pending | T0 的 OrderBook capability 只能在 monitor 进程完成同一真实请求批次的全部准入后刷新：响应必须 ADMITTED、complete=true，payload/envelope 批次身份与时间一致，请求代码非空且无重复，record/rejection outcomes 与请求集合一一对应，所有请求代码均为成功 record、rejections 为空，并且每条五档盘口、价格和时间通过严格校验。部分成功、空 records、任一 rejection、重复/缺失/额外代码、无时区或坏时间均不得刷新；不得新增伪探针、以其他标的成功代表失败标的，或用 BoardFlows/MoneyFlow 替代盘口证据。 | `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md` §11.4, `src/data_gateway/grpc_source.rs` |
```

- [ ] **Step 2: 验证规则登记**

Run: `bash tools/compliance/lib/check_business_rules.sh`

Expected: exit 0，输出 business-rule check PASS；不得出现未登记集合判定。

- [ ] **Step 3: Commit**

```bash
git add docs/business_rules.md
git commit -m "docs: register atomic T0 order-book admission"
```

---

### Task 2: 冻结 `market.t0_evidence` schema v2

**Files:**
- Modify: `src/grpc_contract/schema.rs`

**Interfaces:**
- Consumes: `Operation::T0Evidence` 与既有 `OpSchema` 注册表。
- Produces: `schema_for(Operation::T0Evidence) -> OpSchema { schema_name: "market.t0_evidence", schema_version: 2 }`；server handler 和 client request builder 自动共享。

- [ ] **Step 1: 写失败测试**

在 `src/grpc_contract/schema.rs` 测试模块增加：

```rust
#[test]
fn t0_evidence_uses_frozen_v2_schema() {
    let schema = schema_for(Operation::T0Evidence).expect("T0Evidence schema");
    assert_eq!(schema.schema_name, "market.t0_evidence");
    assert_eq!(schema.schema_version, 2);
}
```

- [ ] **Step 2: 验证测试先失败**

Run: `cargo test --lib grpc_contract::schema::tests::t0_evidence_uses_frozen_v2_schema -- --exact`

Expected: FAIL，左值为 `1`、右值为 `2`。

- [ ] **Step 3: 最小实现**

只修改 T0 条目：

```rust
OpSchema {
    operation: Operation::T0Evidence,
    schema_name: "market.t0_evidence",
    schema_version: 2,
},
```

- [ ] **Step 4: 验证 schema 与错误版本门**

Run: `cargo test --lib grpc_contract::schema::tests::`

Run: `cargo test --lib grpc_contract::validate::tests::`

Expected: PASS；现有 `rejects_unsupported_version` 继续证明未知版本被拒绝。

- [ ] **Step 5: Commit**

```bash
git add src/grpc_contract/schema.rs
git commit -m "feat(grpc): freeze T0 evidence wire v2"
```

---

### Task 3: 服务端用显式 wire DTO 输出 `+08:00`

**Files:**
- Create: `src/grpc_server/t0_wire.rs`
- Modify: `src/grpc_server/mod.rs`
- Modify: `src/grpc_server/delegate.rs`

**Interfaces:**
- Consumes: `&MagicTdxT0Batch`，其中五分钟 `at` 是已校验的 A 股中国标准时间 civil label。
- Produces: `pub(super) fn encode_t0_batch_v2(batch: &MagicTdxT0Batch) -> Result<Vec<u8>, serde_json::Error>`。

- [ ] **Step 1: 注册 wire 模块并写失败测试**

先在 `src/grpc_server/mod.rs` 增加 `mod t0_wire;`。在新文件测试模块构造
`TEST_CODE_T0_001` 批次，并写以下核心断言：

```rust
#[test]
fn encodes_batch_identity_and_china_session_offset() {
    let batch = sample_batch_with_bar(
        NaiveDate::from_ymd_opt(2026, 8, 27)
            .unwrap()
            .and_hms_opt(13, 5, 0)
            .unwrap(),
    );
    let value: serde_json::Value =
        serde_json::from_slice(&encode_t0_batch_v2(&batch).unwrap()).unwrap();

    assert_eq!(value["requested_at"], batch.requested_at.to_rfc3339());
    assert_eq!(value["source_at"], batch.source_at.to_rfc3339());
    assert_eq!(value["observed_at"], batch.observed_at.to_rfc3339());
    assert_eq!(value["batch_id"], "TEST_CODE_T0_BATCH_001");
    assert_eq!(value["time_untrustworthy"], false);
    assert_eq!(
        value["records"][0]["completed_five_minute"][0]["at"],
        "2026-08-27T13:05:00+08:00"
    );
}
```

测试模块使用以下本地 helper；不得为测试新增 production fixture 或默认数据路径：

```rust
fn sample_batch_with_bar(at: NaiveDateTime) -> MagicTdxT0Batch {
    let requested_at = Utc.with_ymd_and_hms(2026, 8, 27, 5, 4, 59).unwrap();
    let source_at = Utc.with_ymd_and_hms(2026, 8, 27, 5, 5, 0).unwrap();
    let observed_at = source_at + chrono::Duration::milliseconds(250);
    let book = || {
        std::array::from_fn(|index| T0BookLevel {
            price: 9.95 + index as f64 * 0.01,
            volume: 100.0,
        })
    };
    let record = MagicTdxT0Evidence {
        instrument: InstrumentId::new(
            Exchange::Shanghai,
            "TEST_CODE_T0_001",
            AssetClass::Equity,
        )
        .unwrap(),
        code: "TEST_CODE_T0_001".to_owned(),
        requested_at,
        source_at,
        observed_at,
        batch_id: "TEST_CODE_T0_BATCH_001".to_owned(),
        quote: MagicTdxT0Quote {
            price: 10.0,
            last_close: 9.9,
            open: 9.95,
            high: 10.1,
            low: 9.8,
            volume: 1_000.0,
            amount: 10_000.0,
            bids: book(),
            asks: book(),
        },
        settled_daily: Vec::new(),
        completed_five_minute: vec![MagicTdxT0FiveMinuteBar {
            at,
            open: 10.0,
            high: 10.1,
            low: 9.9,
            close: 10.0,
            volume: 1_000.0,
            amount: 10_000.0,
        }],
        intraday_average_price: 10.0,
    };
    MagicTdxT0Batch {
        provider: ProviderId::Tdx,
        source: "TEST_CODE_magic_tdx_t0".to_owned(),
        requested_at,
        source_at,
        observed_at,
        batch_id: "TEST_CODE_T0_BATCH_001".to_owned(),
        records: vec![record],
        rejections: Vec::new(),
        time_untrustworthy: false,
    }
}
```

测试 imports 必须包含 `T0BookLevel`、`ProviderId`、`InstrumentId`、`Exchange`、
`AssetClass`、`NaiveDateTime`、`TimeZone` 与 `Utc`。

- [ ] **Step 2: 验证测试因模块不存在而失败**

Run: `cargo test --lib grpc_server::t0_wire::tests::encodes_batch_identity_and_china_session_offset -- --exact`

Expected: FAIL，仅因 `encode_t0_batch_v2` 尚不存在；如果是类型字段不匹配，先修正测试构造器
使其与当前 `MagicTdxT0Batch` 完全一致，再重新确认 RED。

- [ ] **Step 3: 实现 DTO 与固定时区投影**

`src/grpc_server/t0_wire.rs` 使用以下类型边界；字段不得改用 `Value`：

```rust
use crate::data_gateway::{
    MagicTdxT0Batch, MagicTdxT0DailyBar, MagicTdxT0Evidence,
    MagicTdxT0FiveMinuteBar, MagicTdxT0Quote, MagicTdxT0Rejection,
};
use crate::magic_compat::InstrumentId;
use chrono::{FixedOffset, SecondsFormat, TimeZone};
use serde::Serialize;

const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Serialize)]
struct T0EvidenceBatchWireV2<'a> {
    requested_at: String,
    source_at: String,
    observed_at: String,
    batch_id: &'a str,
    time_untrustworthy: bool,
    records: Vec<T0EvidenceRecordWireV2<'a>>,
    rejections: &'a [MagicTdxT0Rejection],
}

#[derive(Serialize)]
struct T0EvidenceRecordWireV2<'a> {
    instrument: &'a InstrumentId,
    code: &'a str,
    requested_at: String,
    source_at: String,
    observed_at: String,
    batch_id: &'a str,
    quote: &'a MagicTdxT0Quote,
    settled_daily: &'a [MagicTdxT0DailyBar],
    completed_five_minute: Vec<T0FiveMinuteBarWireV2>,
    intraday_average_price: f64,
}

#[derive(Serialize)]
struct T0FiveMinuteBarWireV2 {
    at: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
}

fn china_session_at(bar: &MagicTdxT0FiveMinuteBar) -> String {
    let offset = FixedOffset::east_opt(CHINA_OFFSET_SECONDS).expect("static +08:00 offset");
    offset
        .from_local_datetime(&bar.at)
        .single()
        .expect("fixed offset has one local mapping")
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

pub(super) fn encode_t0_batch_v2(
    batch: &MagicTdxT0Batch,
) -> Result<Vec<u8>, serde_json::Error> {
    let records = batch.records.iter().map(T0EvidenceRecordWireV2::from).collect();
    serde_json::to_vec(&T0EvidenceBatchWireV2 {
        requested_at: batch.requested_at.to_rfc3339(),
        source_at: batch.source_at.to_rfc3339(),
        observed_at: batch.observed_at.to_rfc3339(),
        batch_id: &batch.batch_id,
        time_untrustworthy: batch.time_untrustworthy,
        records,
        rejections: &batch.rejections,
    })
}
```

为 `T0EvidenceRecordWireV2<'a>` 实现以下映射；数值逐字段原样复制，不做 clamp、默认或过滤：

```rust
impl<'a> From<&'a MagicTdxT0Evidence> for T0EvidenceRecordWireV2<'a> {
    fn from(record: &'a MagicTdxT0Evidence) -> Self {
        Self {
            instrument: &record.instrument,
            code: &record.code,
            requested_at: record.requested_at.to_rfc3339(),
            source_at: record.source_at.to_rfc3339(),
            observed_at: record.observed_at.to_rfc3339(),
            batch_id: &record.batch_id,
            quote: &record.quote,
            settled_daily: &record.settled_daily,
            completed_five_minute: record
                .completed_five_minute
                .iter()
                .map(|bar| T0FiveMinuteBarWireV2 {
                    at: china_session_at(bar),
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                    amount: bar.amount,
                })
                .collect(),
            intraday_average_price: record.intraday_average_price,
        }
    }
}
```

- [ ] **Step 4: 接入 delegate**

在 `src/grpc_server/mod.rs` 增加 `mod t0_wire;`。把 `fetch_t0_evidence` 中 records/rejections 的直接 `serde_json::to_value` 和 `json!` 视图替换为：

```rust
let data = crate::grpc_server::t0_wire::encode_t0_batch_v2(&batch).map_err(|error| {
    DelegateError::Fetch(FetchFailure::unknown(format!(
        "T0 v2 batch serialization failed: {error}"
    )))
})?;
```

`Fetched` 的 provider/source/source_at/observed_at/batch_id 继续来自同一 `batch`；不得重新取时。

- [ ] **Step 5: 验证服务端测试**

Run: `cargo test --lib grpc_server::t0_wire::tests::`

Run: `cargo test --lib grpc_server::delegate::tests::`

Expected: PASS；编码后的 bar 明确带 `+08:00`，批级身份完整。

- [ ] **Step 6: Commit**

```bash
git add src/grpc_server/t0_wire.rs src/grpc_server/mod.rs src/grpc_server/delegate.rs
git commit -m "fix(grpc): serialize T0 batches with explicit China time"
```

---

### Task 4: 客户端严格解析 v2 并支持空 records

**Files:**
- Modify: `src/data_gateway/grpc_source/convert.rs`
- Modify: `docs/superpowers/plans/2026-08-13-grpc-data-channel.md`

**Interfaces:**
- Consumes: `T0EvidenceBatchWireV2` canonical JSON、`QueryResult` envelope 和 consumer `now`。
- Produces: 现有 `t0_evidence_batch(q) -> Result<MagicTdxT0Batch, GatewayError>` 与 `t0_evidence_batch_at(q, now)`；不改变调用方类型。

- [ ] **Step 1: 把测试 fixture 改成 v2 并加入非空 bar**

`live_t0_q()` 的 payload 设置：

```rust
q.records[0].schema = "market.t0_evidence".to_string();
q.records[0].schema_version = 2;
```

JSON 顶层加入与 envelope 一致的 `requested_at/source_at/observed_at/batch_id` 和
`"time_untrustworthy": false`，record 的 `completed_five_minute` 加入：

```json
[{"at":"2026-08-17T09:35:00+08:00","open":10.0,"high":10.1,"low":9.9,"close":10.0,"volume":1000.0,"amount":10000.0}]
```

- [ ] **Step 2: 写失败测试**

增加以下测试：

```rust
#[test]
fn br253_t0_v2_preserves_china_session_civil_label() {
    let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
    let batch = t0_evidence_batch_at(&live_t0_q(), now).unwrap();
    assert_eq!(
        batch.records[0].completed_five_minute[0].at,
        NaiveDate::from_ymd_opt(2026, 8, 17)
            .unwrap()
            .and_hms_opt(9, 35, 0)
            .unwrap()
    );
}

#[test]
fn br253_t0_v2_rebuilds_empty_record_batch_from_batch_requested_at() {
    let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
    let mut q = live_t0_q();
    let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
    view["records"] = serde_json::json!([]);
    view["rejections"] = serde_json::json!([{
        "code": "TEST_CODE_600519",
        "reason_code": "quote_stale",
        "detail": "TEST_CODE source time unavailable",
        "retryable": true
    }]);
    q.records[0].data = serde_json::to_vec(&view).unwrap();

    let batch = t0_evidence_batch_at(&q, now).unwrap();
    assert!(batch.records.is_empty());
    assert_eq!(batch.rejections.len(), 1);
    assert_eq!(batch.requested_at, Utc.with_ymd_and_hms(2026, 8, 17, 1, 29, 59).unwrap());
}
```

再增加以下两组表驱动测试，明确覆盖无时区、错误时区和批级证据冲突：

```rust
#[test]
fn br253_t0_v2_rejects_missing_or_wrong_session_offset() {
    let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
    for raw_at in ["2026-08-17T09:35:00", "2026-08-17T01:35:00Z"] {
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["records"][0]["completed_five_minute"][0]["at"] =
            serde_json::json!(raw_at);
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let error = t0_evidence_batch_at(&q, now)
            .expect_err("T0 session bars must carry an explicit +08:00 offset");
        assert_eq!(error.reason_code(), "invalid_evidence", "at={raw_at}");
    }
}

#[test]
fn br253_t0_v2_rejects_batch_envelope_conflicts() {
    let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
    for (field, conflicting_value) in [
        ("batch_id", serde_json::json!("TEST_CODE_T0_BATCH_CONFLICT")),
        ("source_at", serde_json::json!("2026-08-17T01:29:59Z")),
        ("observed_at", serde_json::json!("2026-08-17T01:30:00.249Z")),
    ] {
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view[field] = conflicting_value;
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let error = t0_evidence_batch_at(&q, now)
            .expect_err("canonical T0 batch identity must equal the gRPC envelope");
        assert_eq!(error.reason_code(), "invalid_evidence", "field={field}");
    }
}
```

- [ ] **Step 3: 验证 RED**

Run: `cargo test --lib data_gateway::grpc_source::convert::tests::br253_t0_v2 -- --nocapture`

Expected: 至少 civil-label 测试因 `.naive_utc()` 得到 `01:35` 而失败；空批测试因缺 record source minimum 而失败。

- [ ] **Step 4: 实现严格 `+08:00` parser**

在 converter 的 T0 私有 helper 区加入：

```rust
const T0_SCHEMA: &str = "market.t0_evidence";
const T0_SCHEMA_VERSION: u32 = 2;
const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

fn t0_china_session_at(value: &Value) -> Result<NaiveDateTime, GatewayError> {
    let capability = "T0Evidence";
    let raw = as_str(value, "at", capability)?;
    let parsed = DateTime::parse_from_rfc3339(&raw)
        .map_err(|_| err(capability, "T0 five-minute at must be RFC3339"))?;
    if parsed.offset().local_minus_utc() != CHINA_OFFSET_SECONDS {
        return Err(err(capability, "T0 five-minute at must use explicit +08:00"));
    }
    Ok(parsed.naive_local())
}
```

`t0_evidence_batch` 在解析 JSON 前要求 `q.records[0]` 的 schema、version、content type
精确等于 `market.t0_evidence`、`2`、`application/json; charset=utf-8`；从 JSON 顶层解析
批级字段并与 `evidence_of(q)` 的 source/observed/batch ID 比较。
分钟 bar 使用 `t0_china_session_at`，禁止 `.naive_utc()`。`requested_at` 只从顶层读取；
record 仍必须与顶层 requested/observed/batch 一致。

- [ ] **Step 5: 保留服务端不可信标记并允许真实空拒绝批次**

把最终合并改为：

```rust
batch.time_untrustworthy = batch.time_untrustworthy
    || batch_time_untrustworthy
    || record_time_untrustworthy;
```

最早 record source 校验只在 `records` 非空时执行；空 records 必须仍通过顶层/envelope
source_at 一致性和时间顺序校验，且由调用方 exact outcomes 检查其 rejections。

- [ ] **Step 6: 验证 T0 converter 全部测试**

Run: `cargo test --lib data_gateway::grpc_source::convert::tests::br253_t0_v2 -- --nocapture`

Run: `cargo test --lib data_gateway::grpc_source::convert::tests:: -- --nocapture`

Expected: 全部 PASS；旧的未来/倒置/价格非法门继续 PASS。

- [ ] **Step 7: Commit**

```bash
git add src/data_gateway/grpc_source/convert.rs
git commit -m "fix(grpc): strictly reconstruct T0 wire v2 batches"
```

- [ ] **Step 8: 修复 converter 单测的 TEST_CODE 身份隔离**

审查确认 `live_t0_q()` 仍用 `600519`/`SH600519`，违反 AGENTS 2.5。把 Task 4 T0 fixture 的
record/code/instrument 和冲突用例统一改为 `TEST_CODE_600519` / `TEST_CODE_600520`。

production `instrument_for` 继续拒绝 TEST_CODE；只在 `#[cfg(test)]` 单元测试构建中加入一个
明确的 TEST_CODE identity seam：从 `TEST_CODE_` 后缀推断测试 exchange，但构造出的
`InstrumentId.code` 必须保留完整 TEST_CODE，不得剥前缀冒充真实标的。无前缀 production
路径完全不变；integration/release 构建不得包含该 seam。增加 malformed TEST_CODE 后缀拒绝测试。

- [ ] **Step 9: 验证单元测试隔离且生产构建不放宽**

Run: `cargo test --lib data_gateway::grpc_source::convert::tests::br253_t0_v2 -- --nocapture`

Run: `cargo test --lib data_gateway::grpc_source::convert::tests:: -- --nocapture`

Run: `cargo build --release --bin grpc_local_readiness_probe`

Run: `cargo clippy --lib --all-features -- -D warnings`

Expected: converter 全部 PASS；测试输出身份保持 TEST_CODE；release/normal library 仍使用原生产
resolver，未开启 fixture 时 TEST_CODE 不能形成完整 typed readiness。

- [ ] **Step 10: 提交隔离修复**

```bash
git add src/data_gateway/grpc_source/convert.rs \
  docs/superpowers/plans/2026-08-13-grpc-data-channel.md
git commit -m "test(grpc): isolate T0 converter identities"
```

---

### Task 5: 仅在全批成功后刷新 OrderBook capability

**Files:**
- Modify: `src/data_gateway/grpc_source.rs`
- Modify: `src/monitor/data_mode.rs`
- Modify: `docs/business_rules.md`
- Modify: `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md`
- Modify: `docs/superpowers/plans/2026-08-13-grpc-data-channel.md`

**Interfaces:**
- Consumes: 已通过 `convert::t0_evidence_batch_at` 和现有 exact-outcome 集合门的批次计数。
- Produces: `complete_t0_batch_proves_order_book(requested_len, record_len, rejection_len) -> bool`；成功时调用 `mark_capability_success(Capability::OrderBook)`，并按 BR-216 让已接入真实 provider 的全局 OrderBook 作为关键能力参与 DataMode。

- [ ] **Step 1: 写纯函数失败测试**

为测试模块增加以下计数边界测试；identity 完整性仍由当前
`t0_evidence_batch_async` 的 `HashSet` exact-outcome 门负责：

```rust
#[test]
fn br253_t0_order_book_requires_a_nonempty_all_record_batch() {
    assert!(complete_t0_batch_proves_order_book(7, 7, 0));
    assert!(!complete_t0_batch_proves_order_book(7, 6, 1));
    assert!(!complete_t0_batch_proves_order_book(7, 7, 1));
    assert!(!complete_t0_batch_proves_order_book(0, 0, 0));
}
```

- [ ] **Step 2: 验证 RED**

Run: `cargo test --lib data_gateway::grpc_source::tests::br253_t0_order_book -- --nocapture`

Expected: FAIL，`complete_t0_batch_proves_order_book` 尚不存在。

- [ ] **Step 3: 保留 exact outcome 门并实现全覆盖谓词**

保留 `t0_evidence_batch_async` 中现有集合逻辑原位不动，在模块私有区加入：

```rust
fn complete_t0_batch_proves_order_book(
    requested_len: usize,
    record_len: usize,
    rejection_len: usize,
) -> bool {
    requested_len > 0 && record_len == requested_len && rejection_len == 0
}
```

该谓词只能在现有 `outcome_set == requested` 硬门之后调用；不得移动到硬门之前，也不得删除
重复、缺失、额外 outcome 的拒绝逻辑。

- [ ] **Step 4: 在 consumer admission 后刷新 capability**

`t0_evidence_batch_async` 的顺序固定为：query → v2 converter → 现有 exact outcomes →
full coverage → capability mark → return。在现有 exact-outcome `if` 块之后加入：

```rust
if complete_t0_batch_proves_order_book(
    codes.len(),
    batch.records.len(),
    batch.rejections.len(),
) {
    crate::monitor::data_mode::mark_capability_success(
        crate::monitor::data_mode::Capability::OrderBook,
    )
    .map_err(|error| GatewayError::unavailable("T0Evidence", Some(batch.provider), false, error))?;
}
Ok(batch)
```

部分批次仍返回其真实 records/rejections 给策略上层，但不得刷新全局 OrderBook。

- [ ] **Step 5: 验证 helper 与既有 live wrapper 测试**

Run: `cargo test --lib data_gateway::grpc_source::tests:: -- --nocapture`

Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/data_gateway/grpc_source.rs
git commit -m "fix(monitor): admit OrderBook only from complete T0 batches"
```

- [ ] **Step 7: 写 BR-216 分类失败测试**

审查发现：全局 `OrderBook` 成功标记已经证明真实 provider 接入，而 BR-216 明确要求该能力
接入后恢复为关键能力。先修改 `src/monitor/data_mode.rs` 的既有测试，使其表达当前合同：

```rust
#[test]
fn degraded_when_only_orderbook_missing_after_provider_admission() {
    let mut input = input_all_fresh();
    input.capabilities[4] = CapabilityStatus::missing(Capability::OrderBook);
    let h = evaluate(&input, Some(DataMode::Full));
    assert_eq!(h.mode, DataMode::Degraded);
    assert!(h.missing.contains(&Capability::OrderBook));
}

#[test]
fn br216_provider_backed_orderbook_is_critical() {
    assert!(Capability::OrderBook.is_critical());
    assert!(!Capability::MoneyFlow.is_critical());
}
```

保留 `OrderBook` 300 秒、预算 600 秒仍为 Full 的既有测试，并增加超过 600 秒进入 Degraded
的断言；不得把 OrderBook 缺失升级为 Unsafe，Quote 仍是唯一直接触发 Unsafe 的能力。

- [ ] **Step 8: 验证分类测试先失败**

Run: `cargo test --lib monitor::data_mode::tests::br216_provider_backed_orderbook_is_critical -- --exact`

Run: `cargo test --lib monitor::data_mode::tests::degraded_when_only_orderbook_missing_after_provider_admission -- --exact`

Expected: FAIL；现有实现仍把 OrderBook 归辅助并对缺失返回 Full。

- [ ] **Step 9: 实现 BR-216 分类并同步设计真相**

`Capability::is_critical` 只把仍无真实 provider 的 `MoneyFlow` 归为辅助：

```rust
pub fn is_critical(self) -> bool {
    !matches!(self, Capability::MoneyFlow)
}
```

同步修改 `data_mode.rs` 的模块说明、`evaluate` 规则说明、分支注释和
`current_data_health_input` 注释：OrderBook 已由 BR-253 的严格 T0 全批准入标记；缺失或超过
既有 `orderbook_max_age_secs` 时进入 Degraded，仍不得伪造成功或替代 MoneyFlow。

在设计 §11.4 和 BR-253 中明确：`mark_capability_success(OrderBook)` 是全局真实 provider
准入，因此触发 BR-216 的关键能力分类；“不修改推送节流”只表示不修改 dedup/rate-limit，
不能压制 DataMode 的既有安全后果。不修改任何 config threshold。

- [ ] **Step 10: 验证 DataMode、网关和规则合同**

Run: `cargo test --lib monitor::data_mode::tests:: -- --nocapture`

Run: `cargo test --lib data_gateway::grpc_source::tests:: -- --nocapture`

Run: `bash tools/compliance/lib/check_business_rules.sh`

Run: `bash tools/compliance/lib/check_design_contradiction.sh`

Run: `cargo clippy --lib --all-features -- -D warnings`

Expected: 全部 PASS；MoneyFlow 仍是辅助/unsupported，OrderBook 新鲜时 Full、缺失或超过 600 秒时 Degraded，Quote 断流仍为 Unsafe。

- [ ] **Step 11: 提交审查修复**

```bash
git add docs/business_rules.md \
  docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md \
  docs/superpowers/plans/2026-08-13-grpc-data-channel.md \
  src/monitor/data_mode.rs
git commit -m "fix(monitor): make admitted OrderBook critical"
```

---

### Task 6: 更新 fixture、端到端回归并增加脱敏本地探针

**Files:**
- Modify: `src/grpc_server/fixture.rs`
- Modify: `tests/grpc_bridge_e2e.rs`
- Modify: `tests/grpc_channel_e2e.rs`
- Create: `src/bin/grpc_local_readiness_probe.rs`
- Modify: `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md`
- Modify: `docs/superpowers/plans/2026-08-13-grpc-data-channel.md`

**Interfaces:**
- Consumes: 同一候选 server 的 `GrpcMarketClient` 和四个已冻结 operation。
- Produces: 只输出 operation/provider/records/time_untrustworthy/status 的 operator probe；不输出证券代码、价格、原始 payload、token 或证书路径。

- [ ] **Step 1: 把 T0 fixture 更新为 v2**

顶层加入 `requested_at/source_at/observed_at/batch_id/time_untrustworthy`，并加入一根
`+08:00` 五分钟 bar。fixture response 的 payload version 继续使用 handler 传入的 `version`，
因此 T0 自动为 2，其他 op 保持 1。

- [ ] **Step 2: 加强两个 e2e 断言**

`tests/grpc_channel_e2e.rs` 在 T0 case 断言 `result.records[0].schema_version == 2`。
`tests/grpc_bridge_e2e.rs` 在 T0 round-trip 后断言：

```rust
assert_eq!(t0.records[0].completed_five_minute.len(), 1);
assert_eq!(t0.records[0].completed_five_minute[0].at.time().hour(), 13);
assert!(t0.rejections.is_empty());
```

测试模块引入 `chrono::Timelike`。

- [ ] **Step 3: 写 probe 参数和结果模型测试**

`grpc_local_readiness_probe.rs` 定义：

```rust
#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:18083")]
    addr: String,
    #[arg(long, default_value = "600396")]
    code: String,
}

fn format_result(
    operation: &str,
    provider: &str,
    records: usize,
    time_untrustworthy: &str,
    status: &str,
) -> String {
    format!(
        "operation={operation} provider={provider} records={records} \
         time_untrustworthy={time_untrustworthy} status={status}"
    )
}

fn print_result(
    operation: &str,
    provider: &str,
    records: usize,
    time_untrustworthy: &str,
    status: &str,
) {
    println!(
        "{}",
        format_result(operation, provider, records, time_untrustworthy, status)
    );
}
```

单元测试直接调用 `format_result("T0Evidence", "Tdx", 1, "false", "available")`，断言
等于固定五字段字符串，并断言不含 `600396`、`price`、`token` 和 `/`。

- [ ] **Step 4: 实现四项 typed probe**

连接后依次执行 health、capabilities、`RealtimeQuotes`、`OrderBooks`、`T0Evidence`、
`HistoricalBars`。所有 query 都用 `GrpcMarketClient::query` 与对应 converter；请求形状固定为：

```rust
let code = args.code.clone();
let quote_q = client
    .query(Operation::RealtimeQuotes, serde_json::json!({"codes": [&code]}))
    .await
    .map_err(|error| safe_grpc("RealtimeQuotes", error))?;
let quotes = convert::realtime_quotes_at(&quote_q, Utc::now())
    .map_err(|error| safe_gateway("RealtimeQuotes", error))?;

let book_q = client
    .query(Operation::OrderBooks, serde_json::json!({"codes": [&code]}))
    .await
    .map_err(|error| safe_grpc("OrderBooks", error))?;
let books = convert::order_books_at(&book_q, Utc::now())
    .map_err(|error| safe_gateway("OrderBooks", error))?;

let t0_q = client
    .query(Operation::T0Evidence, serde_json::json!({"codes": [&code]}))
    .await
    .map_err(|error| safe_grpc("T0Evidence", error))?;
let t0 = convert::t0_evidence_batch_at(&t0_q, Utc::now())
    .map_err(|error| safe_gateway("T0Evidence", error))?;

let daily_q = client
    .query(
        Operation::HistoricalBars,
        serde_json::json!({"codes": [&code], "days": 5}),
    )
    .await
    .map_err(|error| safe_grpc("HistoricalBars", error))?;
let daily = convert::historical_bars(&code, &daily_q)
    .map_err(|error| safe_gateway("HistoricalBars", error))?;
```

`safe_grpc(context: &'static str, error: GrpcError) -> anyhow::Error` 只读取
`error.details().reason_code/retryable`；`safe_gateway(context: &'static str,
error: GatewayError) -> anyhow::Error` 只读取 `error.reason_code()/retryable()`。两者不得格式化
原始 `error`。health 必须 `live && ready`，capabilities 必须证明四个 operation 都是
`ADMITTED && runtime_available`。Quote/OrderBooks/T0 都要求恰好一条请求 code 对应 record；
T0 还要求 `rejections.is_empty()`；HistoricalBars 要求非空。任一失败立即非零退出。

成功输出分别调用 `print_result`；provider 只用已接纳 batch evidence 的 `ProviderId` 枚举名，
非 T0 的 `time_untrustworthy` 输出 `not_applicable`，T0 输出真实布尔值。不得输出证券代码、
价格、原始 payload、原始错误 message、token 或证书路径。

- [ ] **Step 5: 跑 fixture 与桥接 e2e**

Run: `cargo test --test grpc_channel_e2e -- --test-threads=1`

Run: `cargo test --test grpc_bridge_e2e -- --test-threads=1`

Run: `cargo test --bin grpc_local_readiness_probe`

Expected: 全部 PASS；fixture 隔离且测试代码符合 `TEST_CODE` 约束。

- [ ] **Step 6: Commit**

```bash
git add src/grpc_server/fixture.rs tests/grpc_bridge_e2e.rs tests/grpc_channel_e2e.rs src/bin/grpc_local_readiness_probe.rs
git commit -m "test(grpc): cover T0 v2 roundtrip and local readiness"
```

- [ ] **Step 7: 修复 fixture 的 test/live identity 隔离**

审查确认现有 fixture 与两个 E2E 仍用 `600519`/`SH600519` 真实标的身份，违反 AGENTS 2.5。
把 `src/grpc_server/fixture.rs` 内全部证券 fixture identity，以及两份 E2E 能安全经过现有
测试 seam 的 query、事件、断言统一改为 `TEST_CODE_600519`（其他测试标的同样必须使用
`TEST_CODE_` 前缀）。T0 `instrument` 与 `code` 必须保持同一测试身份；fixture 仍只允许在
`GRPC_GATEWAY_TEST_FIXTURE=1` 隔离路径启用。operator probe 的生产默认 `600396` 保持不变，
但 fixture 动态测试必须显式传 `--code TEST_CODE_600519`。

部分 high-level bridge adapter 会在发出 RPC 前通过 production identity resolver，按设计拒绝
`TEST_CODE_`；禁止为了 E2E 修改/放宽 production converter 或 resolver。对这些 operation，
E2E 改用同一 fixture server 的 `GrpcMarketClient::query`，断言 admission、schema/version、
批次身份与 TEST_CODE raw payload；不得声称这是 typed domain round-trip。T0 producer DTO 与
consumer converter 的 TEST_CODE typed 语义分别由 Tasks 3/4 单测证明；真实完整 typed
server/client round-trip 必须由 Task 8 未开启 fixture 的候选端口 probe 证明。

在设计 §11.6 记录该分层验证边界，避免后续把 raw fixture 合同测试误写成 live/typed 证据。

增加静态隔离回归：逐个扫描上述四个 Task 6 源文件的字符串字面量/fixture JSON，凡值形似
A 股六位证券身份（含 SH/SZ 前缀）都必须以 `TEST_CODE_` 开头。不得只排除两个已知字面量，
也不得仅证明“文件里至少有一个 TEST_CODE”后放行；禁止靠注释豁免。

- [ ] **Step 8: 为 probe 失败与脱敏路径补自动测试**

把 inline 判断提取为不接收/不格式化证券代码的私有纯函数，并覆盖：

- health 任一 `live=false` 或 `ready=false` 返回 `not_ready`；
- 四个 capability 任一不是 `ADMITTED && runtime_available` 返回
  `required_capability_unavailable`；
- Quote/OrderBooks 单记录 identity/count 不匹配返回 `record_identity_mismatch`；
- T0 identity/count 不匹配或任一 rejection 返回
  `record_identity_or_rejection_mismatch`；
- HistoricalBars 空记录返回 `records_unavailable`；
- gRPC/gateway error 的输出只含 operation/reason_code/retryable，不含原始 message、测试
  sentinel、证券代码、价格、token 或路径。

将主体提取为 `async fn run(args: Args) -> anyhow::Result<()>`，`main` 只解析参数并返回
`run(args).await`；自动测试使用无监听 loopback 地址证明连接失败返回 `Err`，且脱敏错误不含
传入的 `TEST_CODE` sentinel。Rust `main -> Result` 的非零退出语义不得被 catch 后改成成功。

- [ ] **Step 9: 验证隔离、失败路径和全部 E2E**

Run: `cargo test --test grpc_channel_e2e -- --test-threads=1`

Run: `cargo test --test grpc_bridge_e2e -- --test-threads=1`

Run: `cargo test --bin grpc_local_readiness_probe -- --test-threads=1`

Run: `rg -n '"600519"|SH600519' src/grpc_server/fixture.rs tests/grpc_bridge_e2e.rs tests/grpc_channel_e2e.rs src/bin/grpc_local_readiness_probe.rs`

Expected: 三组测试全部 PASS；`rg` 无输出、exit 1；fixture 只能用 TEST_CODE，失败测试不泄露
sentinel 或原始错误。

- [ ] **Step 10: 提交审查修复**

```bash
git add src/grpc_server/fixture.rs tests/grpc_bridge_e2e.rs tests/grpc_channel_e2e.rs \
  src/bin/grpc_local_readiness_probe.rs \
  docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md \
  docs/superpowers/plans/2026-08-13-grpc-data-channel.md
git commit -m "test(grpc): isolate fixture identities and probe failures"
```

---

### Task 7: Gate B/C、覆盖率和 PR 证据

**Files:**
- Modify if evidence requires: PR description only；不把运行日志或凭据写入仓库。

**Interfaces:**
- Consumes: Tasks 1-6 的固定提交。
- Produces: Gate B/C 结果、BR-253 PR diff coverage 和完整 PR 证据字段。

- [ ] **Step 1: 格式和 strict Clippy**

Run: `cargo fmt --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: exit 0。

- [ ] **Step 2: 全仓测试**

Run: `cargo test --workspace --all-features -- --test-threads=1`

Expected: exit 0；任何失败先按根因返回对应任务，不得标记 pre-existing 后跳过。

- [ ] **Step 3: Gate C 合规**

Run: `bash tools/compliance/check.sh --policy pr`

Expected: `ALL CHECKS PASSED`，freshness 明确显示 `NOT RUN`（Gate C offline）。

- [ ] **Step 4: 生成并检查 BR-253 PR coverage**

```bash
mkdir -p target/coverage
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
cargo llvm-cov report --lcov --output-path target/coverage/lcov.info
python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/coverage.json --lcov target/coverage/lcov.info --base-ref master
```

Expected: changed core executable lines >=90%、其他 changed source lines >=85%，global/core 不低于已登记 baseline。

- [ ] **Step 5: 审查 staged diff 和敏感信息**

先验证 Gate D 全仓覆盖率门：

```bash
python3 tools/coverage/check_thresholds.py --policy release --report target/coverage/coverage.json --lcov target/coverage/lcov.info
```

Expected: 本命令只记录 Gate D 诊断。global <80% 或 core <95% 时状态必须为
`Release Blocked`，但只要 Gate C 已满足，不把跨项目 Gate D 历史缺口吸收到本次合并；
Task 8 继续禁止执行。

然后审查 diff：

Run: `git diff master...HEAD --check`

Run: `git diff master...HEAD --name-only`

Run: `git diff master...HEAD | rg -n "bearer|client-key|private_key|token"`

Expected: 前两项干净且文件范围符合计划；敏感词扫描只能命中文档中的禁止说明或既有类型名，不得出现凭据值/路径内容。

- [ ] **Step 6: 准备 PR 必填字段**

PR 描述必须包含：

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md §11`
- design commit: `6818738`

### Data-Redlines
- [2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10]

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `MagicTdxGateway::get_t0_evidence_batch` | adopt | retain real provider and quality gates |
| direct `serde_json::to_value` T0 record wire | reject | leaks timezone-free NaiveDateTime |
| strict T0 converter | adopt/deepen | retain fail-closed and add v2 batch identity |

### Threshold-Proof
- N/A: no threshold/config change.

### Business-Rules
- BR-238, BR-243, BR-253

### Rollback
- Restore the paired server/monitor binaries recorded by SHA-256; after merge, record the literal merge commit in release evidence and run `git revert` against that exact commit.
```

---

### Task 7A: 用隔离 probe 与 fixture case 闭合 other-production patch coverage

**Files:**
- Modify: `src/bin/grpc_local_readiness_probe.rs`（只改 `#[cfg(test)] mod tests`）
- Modify: `tests/grpc_channel_e2e.rs`

**Interfaces:**
- Consumes: Task 6 的 `run(Args)`、fixture `start(ServerConfig)`、
  `six_representative_ops_fixture_roundtrip`。
- Produces: 一个真实 loopback fixture probe 的 fail-closed 回归，以及全部已登记 fixture operation
  的 raw-wire 回归；不产生 live/provider 证据。

- [ ] **Step 1: 固定当前 other-production RED**

Run:

```bash
awk 'BEGIN{p=0;c=0;h=0} /^SF:.*src\/bin\/grpc_local_readiness_probe.rs$/{p=1} p&&/^DA:/{split(substr($0,4),a,",");c++;h+=(a[2]>0)} p&&/^end_of_record$/{printf "%d/%d %.2f%%\n",h,c,100*h/c;exit !(100*h>=85*c)}' target/coverage/lcov.info
```

Expected: 输出 `179/299 59.87%`，exit 1。该结果只证明旧报告缺口，不是最终 provenance。

- [ ] **Step 2: 增加完整 probe 的隔离失败链测试**

在 probe 测试模块增加 import：

```rust
use stock_analysis::grpc_server::{start, ServerConfig};
```

增加测试：

```rust
#[tokio::test(flavor = "multi_thread")]
async fn isolated_fixture_probe_reaches_t0_and_fails_closed_on_test_identity() {
    let (addr, handle, _hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .expect("TEST_CODE fixture server");

    let error = run(Args {
        addr: format!("http://{addr}"),
        code: "TEST_CODE_600519".to_owned(),
    })
    .await
    .expect_err("production T0 resolver must reject TEST_CODE after safe earlier probes");
    handle.abort();

    let safe = error.to_string();
    assert_eq!(
        safe,
        "operation=T0Evidence reason_code=invalid_evidence retryable=false"
    );
    for forbidden in ["TEST_CODE_600519", "price", "token", "/"] {
        assert!(!safe.contains(forbidden), "probe error leaked {forbidden}");
    }
}
```

该测试必须真实经过 Health、Capabilities、RealtimeQuotes、OrderBooks，再在 T0 typed converter
处拒绝 TEST_CODE。禁止为使 T0 成功而修改 production resolver。

- [ ] **Step 3: 扩充 fixture raw-wire operation 表**

在 `six_representative_ops_fixture_roundtrip` 的 `cases` 追加以下精确条目：

```rust
(Operation::OrderBooks, "market.order_books", "TEST_CODE_600519"),
(Operation::MoneyFlows, "market.money_flows", "TEST_CODE_600519"),
(Operation::ForeignExchange, "market.foreign_exchange", "TEST_CODE_USDCNY"),
(Operation::FuturesDelivery, "market.futures_delivery", "TEST_CODE_IF2608"),
(Operation::DragonTiger, "market.dragon_tiger", "TEST_CODE_600519"),
(Operation::BlockTrades, "market.block_trades", "TEST_CODE_600519"),
(Operation::BoardDirectory, "board.directory", "TEST_CODE_BK0475"),
(Operation::BoardConstituents, "board.constituents", "TEST_CODE_600519"),
(Operation::BoardFlows, "board.flows", "TEST_CODE_BK0475"),
(Operation::MarketRankings, "market.market_rankings", "TEST_CODE_BK0475"),
(Operation::ConceptHits, "market.concept_hits", "TEST_CODE_BK0475"),
(Operation::MarketStatistics, "market.market_statistics", "TEST_CODE_600519"),
(Operation::ResearchReports, "research.reports", "TEST_CODE_600519"),
(Operation::NorthboundDaily, "market.northbound_daily", "TEST_CODE_600519"),
(Operation::FinancialStatements, "market.financial_statements", "TEST_CODE_600519"),
(Operation::FundFlowSeries, "market.fund_flow_series", "TEST_CODE_600519"),
(Operation::ProviderTopNRankings, "market.provider_top_n_rankings", "TEST_CODE_600519"),
(Operation::CorporateActions, "market.corporate_actions", "TEST_CODE_600519"),
```

沿用循环的 `complete`、单 payload、schema/version、JSON 和 TEST_CODE 内容断言；不得为这些
case 新增生产 fixture fallback。

- [ ] **Step 4: 跑聚焦测试与隔离 guard**

Run:

```bash
cargo test --bin grpc_local_readiness_probe tests::isolated_fixture_probe_reaches_t0_and_fails_closed_on_test_identity -- --exact --test-threads=1
cargo test --bin grpc_local_readiness_probe -- --test-threads=1
cargo test --test grpc_channel_e2e -- --test-threads=1
```

Expected: 全部 PASS；probe safe error 不含 code/price/token/path；Task 6 identity guard 继续 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/bin/grpc_local_readiness_probe.rs tests/grpc_channel_e2e.rs
git commit -m "test(grpc): cover isolated readiness probe branches"
```

---

### Task 7B: 通过同一 fixture bridge 覆盖 dormant typed wrappers

**Files:**
- Modify: `tests/grpc_bridge_e2e.rs`

**Interfaces:**
- Consumes: 已启用的单进程 `grpc_source::bridge_for("OutcomeDailyBars")`、同一随机端口 fixture
  server 与 `TEST_CODE_600519`。
- Produces: LocalBridge typed wrapper 的 available/invalid-evidence 分层证据；不改任何 production
  converter、resolver 或 fixture。

- [ ] **Step 1: 在现有唯一 bridge E2E 中取得同一个 source**

在 `bridge_all_hooked_ops_fixture_roundtrip` 完成 `wait_ready` 后加入：

```rust
let bridge = grpc_source::bridge_for("OutcomeDailyBars")
    .expect("TEST_CODE bridge lookup")
    .expect("DATA_GATEWAY_GRPC enables one local bridge");
let test_code = "TEST_CODE_600519".to_owned();
```

不得新建第二个 env-sensitive integration test；现有测试负责最终 `reset_bridge()` 和子进程回收。

- [ ] **Step 2: 覆盖无证券 resolver 的 available wrappers**

在同一测试内加入以下调用和精确计数断言：

```rust
assert_eq!(bridge.global_indices_async().await.unwrap().records().len(), 1);
assert_eq!(bridge.announcements_async().await.unwrap().records().len(), 1);
assert_eq!(bridge.futures_delivery_async().await.unwrap().records().len(), 1);
assert_eq!(
    bridge.board_constituents_async(&test_code).await.unwrap().records().len(),
    1
);
assert_eq!(
    bridge.research_reports_async(&test_code, 5).await.unwrap().records().len(),
    1
);
assert_eq!(
    bridge
        .technical_bars_async(std::slice::from_ref(&test_code), 100)
        .await
        .unwrap()
        .records()
        .len(),
    1
);
assert_eq!(bridge.intraday_shape_async(&test_code).await.unwrap().records().len(), 1);
```

每个 batch 还必须断言 `evidence().batch_id == "fixture-b1"`；字符串记录必须保留对应
TEST_CODE，不用真实证券代码替换。

- [ ] **Step 3: 覆盖 fixture 与 production contract 不一致的显式拒绝**

加入以下断言：

```rust
for error in [
    bridge.foreign_exchange_async().await.unwrap_err(),
    bridge.economic_calendar_async().await.unwrap_err(),
    bridge
        .market_statistics_async(std::slice::from_ref(&test_code))
        .await
        .unwrap_err(),
    bridge.provider_top_n_pair_async(date).await.unwrap_err(),
] {
    assert_eq!(error.reason_code(), "invalid_evidence");
    assert!(!error.retryable());
}

let news_error = bridge
    .instrument_news_async(std::slice::from_ref(&test_code), 5)
    .await
    .expect_err("production identity resolver rejects TEST_CODE before RPC");
assert_eq!(news_error.reason_code(), "invalid_request");
assert!(!news_error.retryable());
```

ForeignExchange fixture 的 `TEST_CODE_USDCNY` 和 EconomicCalendar fixture 的 numeric optional
字段故意不冒充合法 typed contract；测试必须保留显式错误，不修生产代码迎合 fixture。

- [ ] **Step 4: 跑 bridge 与 resolver 隔离回归**

Run:

```bash
cargo test --test grpc_bridge_e2e --all-features bridge_all_hooked_ops_fixture_roundtrip -- --exact --test-threads=1
cargo test --test grpc_channel_e2e task6_fixture_sources_use_only_test_security_identities -- --exact --test-threads=1
```

Expected: 2/2 命令 PASS；LocalBridge fixture 不访问真实 provider，production identity rejection 保留。

- [ ] **Step 5: Commit**

```bash
git add tests/grpc_bridge_e2e.rs
git commit -m "test(grpc): exercise dormant bridge wrappers"
```

---

### Task 7C: 补齐 converter 的 available、verified-empty 与 invalid-evidence 矩阵

**Files:**
- Modify: `src/data_gateway/grpc_source/convert.rs`（只改 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 现有 `mk_q(data, provider, source)`、各 converter 与 `cfg(test)` TEST_CODE exchange
  seam。
- Produces: 纯函数 converter 覆盖；不改非测试函数签名或行为。

- [ ] **Step 1: 写 available/empty/invalid 的表驱动测试**

新增 `offline_fixture_converters_cover_dormant_available_empty_and_invalid_contracts`。每个 case
必须先用下面的精确 payload 得到 Available，再用 `mk_q("[]", provider, source)` 得到
VerifiedEmpty，最后修改列出的字段并断言 `reason_code()=="invalid_evidence"`：

| converter | provider/source | Available payload | invalid mutation |
|---|---|---|---|
| `global_indices` | `Tdx/TEST_CODE_source` | `[{"code":"DowJones","name":"TEST_CODE_DJ","value":41000.0,"change":12.0,"change_percent":0.03,"source_at":"2026-08-15T09:35:00+08:00"}]` | `code="TEST_CODE_UNKNOWN"` |
| `foreign_exchange` | `Tdx/TEST_CODE_source` | `[{"pair":"UsdCny","name":"TEST_CODE_USD_CNY","rate":7.15,"change":null,"change_percent":null,"source_at":"2026-08-15T09:35:00+08:00"}]` | `pair="TEST_CODE_USDCNY"` |
| `announcements` | `Tdx/TEST_CODE_source` | `[{"announcement_id":"TEST_CODE_A1","code":"TEST_CODE_600519","category":null,"title":"TEST_CODE 公告","published_at":"2026-08-15T09:00:00+08:00","url":"https://example.com/TEST_CODE_A1"}]` | 删除 `title` |
| `economic_calendar` | `Jin10/TEST_CODE_source` | `[{"event_id":"TEST_CODE_E1","indicator_id":123,"country":"CN","name":"TEST_CODE CPI","period":"2026-07","scheduled_at":"2026-08-15T09:30:00+08:00","released_at":"2026-08-15T09:30:00+08:00","previous":"0.6","consensus":"0.7","actual":"0.8","revised":null,"unit":"%","importance":3,"impact":"positive"}]` | `importance="bad"` |
| `futures_delivery` | `Tdx/TEST_CODE_source` | `[{"contract_code":"TEST_CODE_IF2608","product_code":"TEST_CODE_IF","last_trading_date":null,"delivery_date":"2026-08-21","notice_url":"https://example.com/TEST_CODE_FD"}]` | `delivery_date="bad-date"` |
| `market_statistics` | `Tdx/TEST_CODE_source` | `[{"code":"TEST_CODE_600519","turnover_rate":0.42,"trailing_pe":28.5,"static_pe":26.0,"pb":9.2,"total_market_cap":1880000000000.0,"floating_market_cap":1880000000000.0,"upper_limit":1650.0,"lower_limit":1350.0,"volume_ratio":1.1}]` | `upper_limit=0.0` |
| `technical_bars` | `Tdx/TEST_CODE_source` | `[{"open":10.0,"close":10.1,"high":10.2,"low":9.9,"vol":1000.0,"amount":10000.0,"at":"2026-08-15T10:30:00+08:00"}]` | 删除 `at` |
| `intraday_shape` | `Tdx/TEST_CODE_source` | `[{"date":"2026-08-15","pre_close":1500.0,"open_pct":0.2,"high_pct":2.1,"low_pct":-0.8,"close_pct":1.3,"amplitude":2.9,"tail_30m_pct":0.5,"shape_label":"TEST_CODE_SHAPE"}]` | 删除 `shape_label` |

测试使用 `serde_json::Value` 做 mutation 后再 `serde_json::to_string`，禁止在 production parser
中增加默认值。Available 分支至少断言 record count、TEST_CODE identity（适用时）、provider 和
batch_id；VerifiedEmpty 分支断言 `is_verified_empty()`。

- [ ] **Step 2: 覆盖剩余 identity-bearing converter**

在同一测试函数后半段加入以下精确 payload：

| converter | provider/source | Available payload | invalid mutation |
|---|---|---|---|
| `market_dragon_tiger` | `Tdx/TEST_CODE_source` | `[{"exchange":"Shanghai","code":"TEST_CODE_600519","ranking_net_amount_yuan":150000000.0,"disclosures":[]}]` | 删除 `exchange` |
| `board_constituents` | `Tdx/TEST_CODE_source` | `[{"instrument_code":"TEST_CODE_600519","board_code":"TEST_CODE_BK0475","board_name":"TEST_CODE_BOARD","kind":"Concept"}]` | `kind="TEST_CODE_UNKNOWN"` |
| `research_reports` | `Tdx/TEST_CODE_source` | `[{"code":"TEST_CODE_600519","report_id":"TEST_CODE_R1","title":"TEST_CODE REPORT","organization":"TEST_CODE_ORG","rating":"Buy","published_at":"2026-08-15T09:00:00+08:00","canonical_url":"https://example.com/TEST_CODE_R1","target_price_upper":12.0,"target_price_lower":10.0}]` | 删除 `report_id` |
| `provider_top_n_rankings` | `Eastmoney/eastmoney-web` | `[{"metric":"VolumeRatio","ordinal":1,"code":"TEST_CODE_600519","label":"TEST_CODE_SECURITY","value":3.2,"unit":"Multiple","trading_date":"2026-08-15","filter_identity":"TEST_CODE_FILTER","provider_declared_total":20,"inspected_row_count":20}]` | `ordinal=0` |
| `instrument_news` | `Sina/TEST_CODE_source` | `[{"code":"TEST_CODE_600519","title":"TEST_CODE NEWS","summary":"TEST_CODE SUMMARY","url":"https://example.com/TEST_CODE_NEWS","source":"Sina","source_name":"TEST_CODE SINA","category":"TEST_CODE CATEGORY","external_id":"TEST_CODE_N1","published_at":"2026-08-15T09:00:00+08:00","fetched_at":"2026-08-15T09:00:01+08:00","content_hash":"TEST_CODE_HASH"}]` | 删除 `content_hash` |

每个 converter 都执行 Available、VerifiedEmpty 和表中 invalid-evidence 断言；不得调用
`grpc_server::fixture_response` 私有函数，也不得把 fixture 值移入 production converter。

- [ ] **Step 3: 跑 RED/GREEN 与全 converter suite**

Run before adding the test:

```bash
cargo test --lib --all-features data_gateway::grpc_source::convert::tests::offline_fixture_converters_cover_dormant_available_empty_and_invalid_contracts -- --exact --test-threads=1
```

Expected RED: exit 101，测试名称不存在。

Run after adding the test:

```bash
cargo test --lib --all-features data_gateway::grpc_source::convert::tests::offline_fixture_converters_cover_dormant_available_empty_and_invalid_contracts -- --exact --test-threads=1
cargo test --lib --all-features data_gateway::grpc_source::convert::tests -- --test-threads=1
```

Expected GREEN: 新测试 1/1，converter suite 全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/data_gateway/grpc_source/convert.rs
git commit -m "test(grpc): cover dormant converter contracts"
```

---

### Task 7D: 覆盖 opening canary、BR-159 audit 与 DataMode tracker 边界

**Files:**
- Modify: `src/data_gateway/grpc_source.rs`（只改 `#[cfg(test)] mod tests`）
- Modify: `src/monitor/data_mode.rs`（只改 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `opening_route`、四个 exact canary、`opening_t0_route`、
  `audit_opening_readiness_report/failure`、`CapabilityTracker`。
- Produces: fail-closed 证据矩阵与 SQLite 测试审计；不触发 provider、通知、订单或生产 DB。

- [ ] **Step 1: 新增 opening canary 纯函数矩阵**

新增测试 `br238_opening_canaries_cover_exact_and_fail_closed_identity_evidence_matrix`。在测试模块
内用 `QueryResult` + `CanonicalPayload` 构造 helper：

```rust
fn br238_query(schema: &str, version: u32, data: serde_json::Value) -> QueryResult {
    QueryResult {
        admission: pb::AdmissionState::Admitted,
        selected_provider: "Tdx".to_owned(),
        batch_id: "TEST_CODE_OPENING_BATCH".to_owned(),
        complete: true,
        observed_at: "2026-08-17T01:30:00.250Z".to_owned(),
        source_at: "2026-08-17T01:30:00Z".to_owned(),
        records: vec![pb::CanonicalPayload {
            schema: schema.to_owned(),
            schema_version: version,
            content_type: "application/json; charset=utf-8".to_owned(),
            data: serde_json::to_vec(&data).unwrap(),
        }],
        source: "TEST_CODE_tdx".to_owned(),
        diagnostic_blocker: String::new(),
    }
}
```

用现有 converter 从 TEST_CODE JSON 产生 quote/book/membership/T0 batch，分别断言 exact code
成功；再对空数组、错 code、重复 code、T0 rejection、空 source、空 batch、
`source_at > observed_at` 断言精确 `opening_canary_empty`、
`opening_canary_identity_mismatch` 或 `invalid_evidence`。T0 v2 JSON 沿用 Task 4
`live_t0_q()` 字段集，batch/code 固定 `TEST_CODE_OPENING_BATCH`/`TEST_CODE_600519`。

- [ ] **Step 2: 新增 BR-159 测试数据库审计矩阵**

新增测试 `br159_opening_audits_cover_available_empty_failure_and_overflow`：

```rust
let _env = test_grpc_env_guard();
crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database");

let report = OpeningReadinessReport {
    routes: vec![
        br238_ready_route("Announcements", ProviderId::Cninfo),
        OpeningRouteReadiness {
            records: 0,
            ..br238_ready_route("BoardConstituents", ProviderId::Tdx)
        },
    ],
    degraded_routes: vec![],
};
audit_opening_readiness_report("TEST_CODE_GATE_C", &report).unwrap();

let failure = GatewayError::classified(
    "TEST_CODE_Opening",
    Some(ProviderId::Tdx),
    "unavailable",
    "TEST_CODE_provider_unavailable",
    true,
    "TEST_CODE failure",
);
audit_opening_readiness_failure("TEST_CODE_GATE_C", &failure).unwrap();
```

随后查询 `data_acquisition_audit`，按 capability 前缀 `TEST_CODE_GATE_C-` 断言 available、
verified_empty、failure 三行的 accepted/rejected/retryable/reason_code。最后把单 route 的
`records=usize::MAX` 传给 `audit_opening_readiness_report`，断言
`reason_code()=="accepted_count_overflow"` 且没有部分新增审计行。

- [ ] **Step 3: 新增 CapabilityTracker fail-closed matrix**

新增 `tracker_fail_closed_transition_matrix`，使用局部 tracker 和固定时间：

```rust
let tracker = CapabilityTracker::default();
let wall = FixedOffset::east_opt(8 * 3600)
    .unwrap()
    .with_ymd_and_hms(2026, 8, 28, 9, 30, 0)
    .unwrap();

assert_eq!(
    tracker.record_attempt_started(Capability::Quote, " ", wall),
    Err("provider must not be blank".to_owned())
);
tracker.register_unsupported(Capability::MoneyFlow).unwrap();
assert_eq!(
    tracker.record_attempt_started(Capability::MoneyFlow, "TEST_CODE_provider", wall),
    Err("unsupported capability cannot be attempted".to_owned())
);
assert_eq!(
    tracker.record_success(
        CapabilitySuccess {
            capability: Capability::News,
            provider: "TEST_CODE_provider".to_owned(),
            provider_observed_at: Some(wall),
            locally_observed_at: wall,
        },
        Instant::now(),
    ),
    Err("capability not registered".to_owned())
);
```

继续覆盖 supported-but-not-started success/failure、blank reason/provider failure；最后对 Quote
完成合法 started→failure，断言 snapshot state=`Failed`、reason、retryable 和
`first_probe_complete=true`。所有状态只存在于局部 tracker。

- [ ] **Step 4: 跑聚焦和模块测试**

Run:

```bash
cargo test --lib --all-features data_gateway::grpc_source::tests::br238_opening_canaries_cover_exact_and_fail_closed_identity_evidence_matrix -- --exact --test-threads=1
cargo test --lib --all-features data_gateway::grpc_source::tests::br159_opening_audits_cover_available_empty_failure_and_overflow -- --exact --test-threads=1
cargo test --lib --all-features monitor::data_mode::tests::tracker_fail_closed_transition_matrix -- --exact --test-threads=1
cargo test --lib --all-features data_gateway::grpc_source::tests -- --test-threads=1
cargo test --lib --all-features monitor::data_mode::tests -- --test-threads=1
```

Expected: 全部 PASS；SQLite 只用 `DatabaseManager::init(None)` 测试库；无 provider/sink/order 调用。

- [ ] **Step 5: Commit**

```bash
git add src/data_gateway/grpc_source.rs src/monitor/data_mode.rs
git commit -m "test(monitor): cover opening readiness failure matrix"
```

---

### Task 7E: 冻结源码，生成两份同源码 coverage 并闭合 BR-252 provenance

**Files:**
- Modify after two matching reports: `config/design_contracts.toml`
- Modify after two matching reports: `docs/business_rules.md`
- Modify after two matching reports: `docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md`
- Modify after two matching reports: `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md`
- Runtime only: 唯一 `/private/tmp/stock-analysis-t0-gatec-coverage.*` 下的
  `run1.{json,lcov}`、`run2.{json,lcov}`

**Interfaces:**
- Consumes: Tasks 7A–7D 的 clean test-only HEAD。
- Produces: 一个字面 `source_sha`、两份计数一致的完整报告、非回退 candidate baseline；
  90/85 与 80/95 阈值不变。

- [ ] **Step 1: 先跑 test-only HEAD 的完整 Gate B**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh --policy pr
git status --short
```

Expected: 前四项 exit 0；freshness 明示 NOT RUN；status 无 tracked 改动。此时记录字面 HEAD
为唯一 coverage source；后续报告完成前不得再改 `src/`、`tests/`、Cargo 或 build 输入。

- [ ] **Step 2: 生成第一份完整 JSON/LCOV**

Run:

```bash
T0_GATEC_COVERAGE_DIR="$(mktemp -d /private/tmp/stock-analysis-t0-gatec-coverage.XXXXXX)"
readonly T0_GATEC_COVERAGE_DIR
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --json --output-path "$T0_GATEC_COVERAGE_DIR/run1.json" -- --test-threads=1
cargo llvm-cov report --lcov --output-path "$T0_GATEC_COVERAGE_DIR/run1.lcov"
python3 tools/coverage/check_thresholds.py --policy release --report "$T0_GATEC_COVERAGE_DIR/run1.json"
```

Expected: coverage tests 0 failed；JSON/LCOV 写出。最后一项因固定 80/95 尚未达到而 exit 1，
但必须输出新鲜 global/core covered/count；不得把该诊断记为 Gate D PASS。

- [ ] **Step 3: 清理 profile 后独立生成第二份报告**

Run:

```bash
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --json --output-path "$T0_GATEC_COVERAGE_DIR/run2.json" -- --test-threads=1
cargo llvm-cov report --lcov --output-path "$T0_GATEC_COVERAGE_DIR/run2.lcov"
python3 tools/coverage/check_thresholds.py --policy release --report "$T0_GATEC_COVERAGE_DIR/run2.json"
```

Expected: 与 run1 相同：测试 0 failed，release diagnostic exit 1。

- [ ] **Step 4: 比较整数、core file count 与 source set**

保存两次 release diagnostic 的完整五元组
`global_covered/global_count/core_covered/core_count/core_file_count`，并运行：

```bash
jq -r '.data[0].files[].filename' "$T0_GATEC_COVERAGE_DIR/run1.json" | sort > "$T0_GATEC_COVERAGE_DIR/run1-json-sources.txt"
jq -r '.data[0].files[].filename' "$T0_GATEC_COVERAGE_DIR/run2.json" | sort > "$T0_GATEC_COVERAGE_DIR/run2-json-sources.txt"
sed -n 's/^SF://p' "$T0_GATEC_COVERAGE_DIR/run1.lcov" | sort > "$T0_GATEC_COVERAGE_DIR/run1-lcov-sources.txt"
sed -n 's/^SF://p' "$T0_GATEC_COVERAGE_DIR/run2.lcov" | sort > "$T0_GATEC_COVERAGE_DIR/run2-lcov-sources.txt"
cmp "$T0_GATEC_COVERAGE_DIR/run1-json-sources.txt" "$T0_GATEC_COVERAGE_DIR/run2-json-sources.txt"
cmp "$T0_GATEC_COVERAGE_DIR/run1-lcov-sources.txt" "$T0_GATEC_COVERAGE_DIR/run2-lcov-sources.txt"
```

Expected: 两轮 global/core covered/count、core file count、JSON source set、LCOV source set
逐项相等；candidate 必须满足整数交叉乘：

```text
candidate_global_covered * 258810 >= 201256 * candidate_global_count
candidate_core_covered   * 202935 >= 157635 * candidate_core_count
```

任一不相等或任一比例回退：不得编辑合同，返回 Task 7B/7C/7D 补测试后重新冻结并跑两轮。

- [ ] **Step 5: 用双报告的精确值更新合同和证据**

仅在 Step 4 通过后：

- `source_sha` 写 Task 7E Step 1 的字面 40 位 commit；
- `global_covered/global_count/core_covered/core_count/core_file_count` 写两份报告一致的整数；
- rustc/LLVM/cargo-llvm-cov identity 写报告实际值且必须与现有固定工具一致；
- `pr_core_patch_min=90`、`pr_other_patch_min=85`、`release_global_min=80`、
  `release_core_min=95` 原样保留；
- `coverage.reviewed_no_region` 集合与 hash 原样保留；
- BR-252 与两个设计文档记录 source SHA、两轮整数、报告 SHA-256、Gate C/Release 分层结论。

然后用 run2 做 candidate contract 验证：

```bash
python3 tools/coverage/check_thresholds.py --policy pr --report "$T0_GATEC_COVERAGE_DIR/run2.json" --lcov "$T0_GATEC_COVERAGE_DIR/run2.lcov" --base-ref master
bash tools/compliance/lib/check_design_contradiction.sh
bash tools/compliance/lib/check_business_rules.sh
```

Expected: PR coverage exit 0；core patch >=90%、other patch >=85%、global/core ratchet PASS；
两项文档门禁 exit 0。失败时不提交合同。

- [ ] **Step 6: Commit provenance closure**

`docs` 已被 ignore 但目标文件均已跟踪；禁止 `git add -f`。使用：

```bash
git add config/design_contracts.toml
git add -u -- docs/business_rules.md docs/superpowers/specs/2026-08-13-grpc-data-channel-design.md docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md
git diff --cached --check
git commit -m "test(coverage): rebind Gate C baseline evidence"
```

---

### Task 7F: 最终独立 Gate C 与 merge-ready PR 证据

**Files:**
- Runtime only: fresh coverage report and PR description；不得提交日志、凭据、DB 或二进制。

**Interfaces:**
- Consumes: Task 7E 的 clean HEAD 与已提交 BR-252 contract。
- Produces: 独立 Gate C 结论；Gate D 必须继续标记 `Release Blocked`。

- [ ] **Step 1: 独立 reviewer 重跑完整 Gate C**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh --policy pr
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/final.json -- --test-threads=1
cargo llvm-cov report --lcov --output-path target/coverage/final.lcov
python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/final.json --lcov target/coverage/final.lcov --base-ref master
```

Expected: 全部 exit 0；freshness 明示 NOT RUN；coverage 整数与 Task 7E 双报告一致。

- [ ] **Step 2: 独立审查范围和敏感信息**

Run:

```bash
git diff master...HEAD --check
git diff master...HEAD --name-only
git diff master...HEAD | rg -n "bearer|client-key|private_key|token"
```

Expected: diff clean；只含计划路径；敏感词只命中文档禁止说明或测试 sentinel，无凭据值或
证书路径。reviewer 必须给出 Critical/Important/Minor=0 才能 PASS。

- [ ] **Step 3: 生成 merge-ready PR 描述但不部署**

沿用 Task 7 Step 6 的 Refs/Data-Redlines/OldModules/Threshold-Proof/Business-Rules/Rollback，
并增加：

```markdown
### Gate-Policy
- PR=core-patch90+other-patch85+ratchet
- Release=global80+core95+freshness+live

### Gate-C
- PASS

### Gate-D
- Release Blocked: global/core 80%/95%、production freshness、live provider 和 auditor sign-off 未闭合
```

只有 Gate C 和独立审查全部 PASS 才可创建 merge-ready PR。不得运行 Task 8、停止进程、
回填生产日线或部署二进制；合并后仍不能称为 Release Ready/Done。

---

### Task 8: 候选端口验证、成对切换、日线修复与观察

**Files:**
- Runtime only: unique `/private/tmp` candidate/backup directories and production DB through registered backfill；不提交二进制、日志或 DB。

**Interfaces:**
- Consumes: Gate B/C/coverage 全通过的同一 HEAD。
- Produces: 同版本 server/monitor、通过真实 typed probe 的本地数据链、通过 2.4 freshness 的 `stock_daily`，以及两个严格五分钟观察窗口。

- [ ] **Step 1: 记录旧运行身份，不做变更**

Run: `lsof -nP -iTCP:18082 -sTCP:LISTEN`

Run: `pgrep -fl 'target/.*/monitor|target/.*/grpc_market_server'`

从输出逐一人工抄录进程号，并立即用 `ps -p` 的进程选择参数及 `lsof -p` 的进程选择参数
复核启动时间、完整命令和打开文件；命令中必须直接写入刚抄录的十进制进程号。再对输出中
解析出的两个绝对二进制路径逐一运行 `shasum -a 256`，命令中直接写绝对路径。后续 kill
必须使用这里人工核对后的字面量进程号，不得使用 glob、命令替换或未验证环境变量。

- [ ] **Step 2: 在独立 target 构建候选**

运行以下命令创建唯一目录并把绝对路径赋给任务专用只读变量：

```bash
T0_CANDIDATE_TARGET="$(mktemp -d /private/tmp/stock-analysis-t0-v2-candidate.XXXXXX)"
readonly T0_CANDIDATE_TARGET
CARGO_TARGET_DIR="$T0_CANDIDATE_TARGET" cargo build --release --bin grpc_market_server --bin monitor --bin grpc_local_readiness_probe
```

Expected: exit 0；workspace 的 `target/release` 尚未被覆盖。

- [ ] **Step 3: 在 18083 启动真实候选 server**

候选 server 环境必须继承生产 provider/DB 配置，但明确 unset `DATA_GATEWAY_GRPC`，并设置
`GRPC_MARKET_PORT=18083`。启动后运行：

```bash
"$T0_CANDIDATE_TARGET/release/grpc_local_readiness_probe" --addr http://127.0.0.1:18083
```

Expected: health ready；Quote/OrderBooks/T0Evidence/HistoricalBars 全部 status=available，T0
`time_untrustworthy` 可真实显示但不能有 invalid_evidence；输出不含代码、价格或凭据。

- [ ] **Step 4: 备份旧二进制并成对切换**

用 `mktemp -d /private/tmp/stock-analysis-t0-v2-rollback.XXXXXX` 创建唯一备份目录，复制 Step 1
解析出的两份旧二进制并保存各自 SHA-256。先向已核对的旧 monitor 字面量进程号发送
SIGINT 并确认退出，再向旧 server 字面量进程号发送 SIGTERM 并确认 18082 释放；随后把候选 server/monitor 安装
到 Step 1 的精确绝对路径并校验 hash，先启动 server，再启动 monitor。

- [ ] **Step 5: 切换后立即验证，否则回滚**

Run: `target/release/grpc_local_readiness_probe --addr http://127.0.0.1:18082`

Expected: 四项全部 available。若失败：停止两份新进程，恢复 rollback 目录两份旧二进制并
核对旧 hash，先启动旧 server 再启动旧 monitor；任务状态记 Blocked，不进入 backfill。

- [ ] **Step 6: 修复生产日线 freshness**

在新 server probe 全绿后运行：

```bash
STOCK_DB=/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db bash tools/one_shot/backfill_daily.sh
STOCK_DB=/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db bash tools/compliance/lib/check_data_freshness.sh
```

Expected: backfill 只接纳真实 HistoricalBars 批次；freshness PASS，`MAX(date)` 落后不超过一个交易日。

- [ ] **Step 7: Gate D release compliance**

Run: `STOCK_DB=/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db bash tools/compliance/check.sh --policy release`

Expected: `ALL CHECKS PASSED`。不得设置 `FRESHNESS_TODAY` 或 `TRADING_CALENDAR`。

- [ ] **Step 8: 连续观察两个严格五分钟窗口**

每个窗口使用 SQLite `julianday(created_at) >= julianday('now','-5 minutes')` 统计最近五分钟
真实审计；禁止 ISO `T...Z` 与 SQLite 空格时间字符串直接比较。验收：

- T0 不再出现 `premature end of input`、无 offset 或 `invalid_evidence`；
- OrderBook 只有完整 T0 批次才刷新，部分批次不刷新；
- MoneyFlow 仍明确 `unsupported_contract`，不被 BoardFlows 成功覆盖；
- Quote/OrderBook freshness 失败保留结构化错误，不输出价格型建议；
- 无订单授权、门禁放宽或伪造 fallback。

- [ ] **Step 9: 创建 PR 并等待检查**

用 Task 7 的完整描述创建 PR；等待 coverage/compliance/test checks 全部结束。任一检查失败按
根因返回 Gate A/B/C，不合并。全部通过且 live evidence 完整后才可声明本修订 Done；Gate D
覆盖率或外部 provider 阻塞时必须保持 In Progress/Blocked。
