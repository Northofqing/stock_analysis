# `magic-*` 依赖彻底删除设计

**日期：** 2026-08-30
**状态：** 用户已批准
**上位设计：** `docs/superpowers/specs/2026-08-23-provider-host-repository-split-design.md`

## 1. 目标

将 `stock_analysis` 收敛为纯 gRPC 市场数据客户端。仓库不再编译、链接或直接调用
`magic-market-data-rs` 中的 provider、router、composition 或 core crate，也不再提供生产
`grpc_market_server`。

完成后：

- `Cargo.toml`、`Cargo.lock` 和默认 Cargo 依赖图中没有 `magic-*` crate 或其 Git URL；
- Rust 源码中没有 `magic_*_rs`、`magic_market_core`、`magic_market_router` 或
  `magic_market_composition` 路径；
- 不存在 `magic-gateway` feature、相关条件编译分支或本地 provider fallback；
- 生产市场数据只能通过现有 gRPC 客户端进入；远端不可用时显式失败；
- 现有 evidence、freshness、canonical identity、交易、决策和审计语义保持不变。

## 2. 范围

### 2.1 删除

- `Cargo.toml` 中 14 个直接 `magic-*` Git 依赖和 `magic-gateway` feature；
- `Cargo.lock` 中因此变得不可达的包；
- `src/grpc_server/**`、`src/bin/grpc_market_server.rs` 和只用于直连 provider 的 probe binary；
- `src/data_gateway/**` 中所有 provider client、provider router 和本地 fallback 实现；
- `src/magic_compat/**` 这个过渡命名及其 feature 双实现；
- `build.rs`、`build_support/**` 中对 `magic-tdx-rs` lock revision 的提取与校验；
- 固定上游 revision、provider release 一致性和本地直连行为的测试；
- 已失效的 M5/本地 server 构建说明和 provider fallback 开关。

### 2.2 保留

- `src/grpc_client/**`、`src/grpc_contract/**`、protobuf 合同和客户端 DTO 转换；
- `src/data_gateway/grpc_source.rs` 及其 operation、capability、错误分类和 fail-closed 行为；
- provider-neutral 的 evidence、freshness、identity、admission 和领域校验；
- 只在测试中使用、且不依赖外部 provider crate 的 gRPC fixture server；
- 数据库和 wire contract 中的稳定 provider 身份，例如 `"magic-tdx"`。这些字符串是历史数据及
  远端合同身份，不是 Rust 包引用，不得因本次清理改写；
- 当前工作树中 `grpc_source.rs` 对 `no_current_reports` 的保真映射及其测试。

### 2.3 不做

- 不复制或 vendor 上游 provider 源码；
- 不在本仓库创建新的 provider 服务端；
- 不改变信号、排序、仓位、下单、新鲜度或证据准入规则；
- 不修改现有数据库中的 provider/contract identity；
- 不用空成功、默认值、假数据或静默回退替代 gRPC 失败；
- 不夹带当前工作树中的 selection activation 修改。

## 3. 目标结构

```text
external provider host
        │
        │ versioned gRPC contract
        ▼
grpc_client / grpc_contract
        │
        ▼
data_gateway::grpc_source
        │ transport/error conversion
        ▼
provider-neutral admission + market domain types
        │
        ▼
monitor / selection / decision / trading / audit
```

`stock_analysis` 不再拥有图中 external provider host 的实现。任何业务路径若仍需要本地
provider client 才能工作，应当改接已经存在的 gRPC operation；若合同没有对应 capability，
该路径必须返回 `OperationUnsupported` 或现有等价的显式失败，不能保留隐藏 fallback。

## 4. 本地域类型

当前 `src/magic_compat/**` 在 feature 关闭时已经提供 provider-neutral 的本地类型。删除依赖时：

1. 将这些本地类型迁入不带上游品牌含义的 `src/market_domain/**`；
2. 删除 feature 开启时重导出上游真实类型的分支；
3. 将业务代码统一改为 `crate::market_domain::*`；
4. 保持字段、枚举、serde 表示和既有 wire identity 不变；
5. 删除只用于比较本地镜像与上游类型的双模式测试，保留本地域类型自己的序列化合同测试。

该迁移只改变类型所有权和导入路径，不授权重塑领域模型。

## 5. Gateway 收敛规则

每个引用 `magic-*` crate 的 gateway 按以下规则处理：

- 保留 provider-neutral request、result、evidence、admission 和错误类型；
- 保留 gRPC DTO 到领域结果的转换与校验；
- 删除 provider client 构造、source router、blocking adapter 和真实网络 canary；
- 删除 `#[cfg(feature = "magic-gateway")]` 块及与其配对的运行时分支；
- 公共 API 若仍被业务调用，改为唯一调用 gRPC bridge；
- 仅供本地 server/probe 调用且没有客户端消费者的 API 连同测试删除；
- gRPC 不可用、能力未发布或 evidence 不合格时沿现有 `GatewayError`/`GrpcSourceError`
  路径显式失败。

不允许通过保留空的 `magic-gateway` feature、永久为假的 `cfg` 块或注释掉代码来满足删除要求。

## 6. 构建与合同

`build.rs` 继续负责从现有 protobuf 合同生成 tonic/prost 代码，但删除以下职责：

- 读取 `Cargo.lock`；
- 查找 `magic-tdx-rs`；
- 注入 `MAGIC_TDX_DEPENDENCY_REVISION`；
- 校验 Git source revision。

需要来源 identity 的 benchmark/evidence 代码改用合同自身的稳定版本或远端响应携带的 source，
不得再从已删除的 Rust 依赖推导版本。为测试 gRPC client 可以继续生成 server trait，但生产
server module 和 binary 必须删除。

## 7. 测试与验收

### 7.1 静态验收

- `Cargo.toml` 不含 `magic-*` dependency 或 `magic-gateway`；
- `Cargo.lock` 不含 `magic-*` package 或 `magic-market-data-rs.git`；
- `cargo tree --locked` 不含 `magic-*`；
- Rust 代码不含已删除 crate 的标识符；
- 不存在生产 `grpc_market_server` target 或 provider-only probe target；
- 不存在 `#[cfg(feature = "magic-gateway")]`。

provider identity 字符串和描述历史决策的归档文档不计为 crate 残留。活跃构建说明必须更新为
纯客户端事实。

### 7.2 行为验收

- gRPC 成功响应仍经过完整 evidence/admission 校验；
- `no_current_reports` 等已知 reason code 保真；
- transport unavailable、contract mismatch、operation unsupported 和 rejected evidence 均 fail closed；
- 业务调用不会尝试本地 provider fallback；
- protobuf 生成、library、主要生产 binary 和相关 integration tests 编译通过。

### 7.3 工作树保护

实施前已有的以下修改属于用户，迁移必须保留：

- `config/selection/selection_activation.v1.json`；
- `src/data_gateway/grpc_source.rs` 中 `no_current_reports` 映射及测试；
- 根目录现有未跟踪架构与规划文件。

只暂存本次迁移明确修改的路径，不使用 reset、checkout 或全量格式化覆盖用户改动。

## 8. 完成定义

只有静态验收、构建验收和相关测试均以当前工作树的新鲜命令返回成功，才能声明完成。若某个
业务路径缺少对应远端 capability，删除本地 fallback 后应保留明确失败，并在交付说明中列出，
不能因此恢复 `magic-*` 依赖。
