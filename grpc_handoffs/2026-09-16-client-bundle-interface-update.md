# 2026-09-16 client-bundle 接口更新审计

日期：2026-09-16。新版公开 bundle 位于 `R=/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle`；当前消费代码位于隔离开发树 `W=R/.worktrees/push-reliability-20260905`。

本文是静态接口差异与兼容适配交接，不是接口已修复、服务端已部署、真实 RPC 已通过或生产验收。审计没有读取 Bearer token、环境文件或私钥，没有调用网络/RPC、数据库、monitor、Cargo，也没有修改 bundle 或 Rust 源码。

## 结论

新版不是仅文档更新。公开 Proto 从 60 个 MarketData RPC 追加到 63 个，并追加 Health/Listener 运行观测、构建身份和结构化 Provider 尝试链。W 当前不能直接替换新版 `market.proto`：`build.rs` 仍把本地私有 `ChainBatch`、`BenchmarkBars` 固定追加为 Operation 61、62，与新版公开 Operation 61、62 精确撞号。

这个冲突不能用“换成 64/65”直接结案。`ChainBatch` 和 `BenchmarkBars` 已有真实本地调用链；旧本地服务响应及错误回执仍可能返回 61/62。公开新版服务的 61/62 又分别具有不同语义。必须先选择以下一种明确迁移合同：

1. 为两项私有 RPC 分配不与公开合同冲突的编号，并同步重建、部署本地服务和客户端；或
2. 把私有 RPC 移到独立 package/service；或
3. 保留旧本地服务时，在 `LocalBridgeV1` 的方法级响应/错误边界显式把 legacy raw 61/62 解释为 `ChainBatch`/`BenchmarkBars`，同时 `ExternalV1` 始终按公开 61/62 解释。

当前仓库有 local/external 连接与请求构造分层，但没有独立于生成 Proto `Operation` 的 domain-op 映射，故第三种方案需要新增窄适配层，不能靠现有代码自动完成。

## 输入版本与完整性

新版 `README.md` 声明 bundle `2026-09-15.1`、source commit `b0218b088cdbed2e73db006aa49b03c361eb95d1`、63 个 MarketData RPC。公开 manifest 已用 `shasum -a 256 -c manifest.sha256` 本地校验，七项全部通过。部署私有的连接材料不在公开 manifest 中，也未纳入本次读取。

| 输入 | 行数 | SHA-256 | 说明 |
| --- | ---: | --- | --- |
| W `client-bundle/market.proto` | 339 | `8730bce3c20e170cf8f58047336ae06d3a5e9080d81568dee71e7b0882063332` | 旧 Proto；无随附公开文档副本 |
| R `client-bundle/market.proto` | 391 | `2f2037a00250b90bbd30525be2e7e9ad2e64c6dd30defc150642b9f993896a7d` | 新 Proto |
| R `client-bundle/README.md` | 18 | `69a1b54d06ddd6186b6f15d191f641cc845cb7c353db6f28c958c5ab92feae97` | bundle 摘要 |
| R `client-bundle/manifest.sha256` | 7 | `c4853e39d5d83db11c467c5ec8d3e7c9e15bdd3de4130fa5e853edac47393205` | 公开文件清单 |
| R `client-bundle/grpc-external-api.md` | 1069 | `a4bbff088cd7e6dbc2b97084f155994a0eaef30b79720351d267f584e70448a8` | 公共接口与状态语义 |
| R `client-bundle/grpc-derived-products.md` | 262 | `d53bcfba71ea1bb58f77f41b50b18343e70438a53817ca3d304947c1fc2621db` | 56～60 派生合同 |
| R `client-bundle/tdx-public-security-profile.md` | 79 | `b23ec602e72a8b06a75c142d976e447685a57bb206bf0ae525945b0dbe0f2424` | TDX 公开安全资料范围 |
| R `client-bundle/unadmitted-provider-routes.md` | 44 | `26e026231e3631806b43da63d7417278aa207458979200a99a8c3481d2634aad` | 未准入来源与替代边界 |
| R `client-bundle/bundle-metadata.json` | 12 | `dbc687d1db04303972219e4fc3dbbdc608cfa4c7bce367222a14839c47eb0283` | 版本、commit、schema 版本 |

W 中不存在旧版 README/manifest/接口文档副本，因此本文只对 Proto 做逐行新旧 diff；公开文档只记录本次实际读取的新版摘要，不伪造不存在的旧文档 SHA 或逐文档差异。

## 精确 Proto 变化

### 1. 三个公开 Operation 与 RPC

新版在 `market.proto:67-69` 追加：

| 数字 | Operation | RPC（`market.proto:377-379`） | 请求/记录 schema v1 |
| ---: | --- | --- | --- |
| 61 | `CURRENT_AUCTION_OBSERVATIONS` | `CurrentAuctionObservations` | `magic.market.current_auction_observations.request` / `magic.market.current_auction_observation` |
| 62 | `ECONOMIC_RELEASE_OBSERVATIONS` | `EconomicReleaseObservations` | `magic.market.economic_release_observations.request` / `magic.market.economic_release_observation` |
| 63 | `ECONOMIC_RELEASE_SCHEDULE` | `EconomicReleaseSchedule` | `magic.market.economic_release_schedule.request` / `magic.market.economic_release_schedule_entry` |

具体语义：

- `CurrentAuctionObservations` 要求 1..N 个明确 instrument 和 `stage=live|final`。记录可给 `auction_volume_ratio`，但允许为空；不提供 `trading_date`，批次及逐条 `source_at` 均为空。`auction_unmatched` 是有符号、单位未公开的 provider-native 单值，不能拆成买/卖未匹配量。见公开文档 `grpc-external-api.md:703-756`。
- `EconomicReleaseObservations` 明确把 `limit` 和可选 `country` 放进 wire；空结果只证明当前混合快讯窗口无结构化 type-1 行，不证明某日或未来日历为空。见 `grpc-external-api.md:758-804`。
- `EconomicReleaseSchedule` 明确传 inclusive `start/end` 和 `limit`，返回 FRED 的 release ID/name/date；只有日期，没有具体发布时间，记录和批次 `source_at` 都为空。见 `grpc-external-api.md:806-836`。
- 原 `EconomicCalendar` 没有被这两项替换，仍 fail-closed/诊断；调用方必须按业务语义显式选择 observations 或 schedule。见 `grpc-external-api.md:490-517`、`unadmitted-provider-routes.md:13`。

### 2. Health 构建身份与运行观测

新版 `HealthResponse` 在 `market.proto:140-141` 追加：

- `observability=5`：进程启动毫秒、单调 uptime、query started/succeeded/failed/cancelled/in-flight/rejected/timed-out、累计/最大耗时微秒、unary/blocking 并发上限和可用 permit（定义见 `market.proto:152-168`）。这些是进程生命周期聚合值，不是 Provider 时间、数据 evidence 或准入结论。
- `build_identity=6`：`service_version`、`source_revision`、descriptor `contract_sha256`、`binary_sha256` 和 `identity_error`（定义见 `market.proto:144-150`）。公开文档要求 monitor/探针/bundle 比较这些值，不能仅凭进程名或源码目录猜运行制品（`grpc-external-api.md:250-261`）。

W 当前外部连接门只检查 `live && ready`，随后查 capability；没有核对 build identity（`src/data_gateway/grpc_source.rs:3017-3039`、`:912-933`）。这是可安全追加的客户端校验，但在获得真实 HealthResponse 前不能声明 GD-001 已解决。

### 3. Listener 聚合状态

新版 `ListenerStatusResponse` 在 `market.proto:244-251` 追加 `replay_oldest`、replay event/bytes、active subscribers、agent connections/disconnects、published events、replay evictions。它们同样是 append-only 观测值。W 的 `get_listener_status` 只返回生成类型（`src/grpc_client/client.rs:461-478`），当前没有投影或判定逻辑；旧客户端忽略未知字段是 wire-safe，但若要使用必须定义“观测”而非“业务证据”的存储/告警语义。

### 4. 有序 Provider 尝试链

新版 `ErrorDetail.provider_attempts=11`（`market.proto:170-190`），每项只有 ordinal、provider、闭合 outcome/reason code、retryable、terminal。公开合同限制最多 16 项，只用于有序路由失败；自由文本、URL、响应体和凭据不得进入数组（`grpc-external-api.md:944-955`）。

W 能解码新版 message，但在 wire → 本地 `ErrorDetail` 转换时只复制旧字段，主动丢弃 `provider_attempts`（`src/grpc_client/errors.rs:338-390`）。新增字段不会改变当前重试决策，但丢失了路由尝试证据；最小适配应使用有界、闭合的本地类型保存它，并为 standard details/trailer 一致性、未知 provider/outcome/reason、超过上限和持久化恢复增加测试。它不能修复 GD-010 的非 gRPC 搜索聚合器，因为两者是不同调用层。

### 5. 现有 RPC 的路由状态变化

`2026-09-15.1` 还把 `HithinkFinance` 注册到现有 `RealtimeQuotes`，未指定 Provider 时使用有界并发竞速、首个非空结果胜出；顶层 selected provider 与逐条 evidence 必须保留真实来源，THS 缺失的逐条 `source_at` 保持空（`grpc-external-api.md:1005-1007`）。这没有 Proto 字段变化，但会改变默认路由的来源分布。任何消费方若硬编码“RealtimeQuotes 默认必为某 Provider”都需审计；真实 provider 竞速结果仍待运行证据。

## P0：本地扩展 61/62 冲突

W `build.rs:49-77` 精确追加：

- `OPERATION_CHAIN_BATCH = 61`、RPC `ChainBatch`；
- `OPERATION_BENCHMARK_BARS = 62`、RPC `BenchmarkBars`。

追加逻辑只按“完全相同声明行是否存在”判断（`build.rs:94-127`），不会发现“相同数字、不同枚举名”。把新版 Proto 放入 W 后会同时保留公开 61/62 和私有 61/62，代码生成前即形成非法重复枚举值。`grpc_contract/ops.rs:1-3,70-71,122-152` 还冻结了 0..=62、共 40 个 implemented op 的假设。

两项私有 RPC 并非死代码：

- `ChainBatch`：`review/catalyst_review.rs:177-190` → `data_gateway/grpc_source.rs:4006-4010` → `:3949-3966` → local `query_op`；请求 schema 冻结在 `grpc_contract/schema.rs:209-214`。
- `BenchmarkBars`：`data_gateway/benchmark.rs:206-215` → `data_gateway/review.rs:616-627` → `data_gateway/grpc_source.rs:3222-3226,2891-2950`；请求 schema 冻结在 `grpc_contract/schema.rs:215-219`，错误回执还在 `grpc_source.rs:480-505` 精确要求 `error_detail.operation == Operation::BenchmarkBars`。

它们不属于新版公开 bundle：新 Proto、公开文档和 metadata 均未声明这两个名称。

### 已有隔离能保证什么

- `QueryRequest` 不包含 operation 数字；本地 `build_query_request` 只放 context/provider/payload（`grpc_client/envelope.rs:34-57`）。操作由 RPC method path 选择。
- `GrpcSource` 分离 local `query_op` 与 external `query_external_op`（`data_gateway/grpc_source.rs:2712-2728,3044-3072`）；`GrpcMarketClient` 也有 `LocalBridgeV1`/`ExternalV1` profile（`grpc_client/client.rs:52-56,602-615`）。
- ExternalV1 请求构造是封闭 allowlist，未交付的 operation 在发送前拒绝（`grpc_client/external_v1.rs:25-116`）。因此现有路径不会把 `ChainBatch`/`BenchmarkBars` 发给新版公开服务。

### 仍缺少什么

- 全仓 method-name、schema、router 和错误判断都直接使用生成的 Proto `Operation`，没有独立 domain-op enum。
- 普通响应会精确比较 `resp.operation == expected_operation as i32`（`grpc_client/envelope.rs:75-94`）。若客户端把私有 operation 改号而旧本地服务仍回 61/62，会 fail-closed 为 operation mismatch。
- `BenchmarkErrorDetail` 也精确比较生成枚举数字；仅正常响应做兼容仍不够。
- 新增 3 个 RPC 后，所有测试用 `MarketDataService` 实现还需补齐生成 trait 方法；新增 Health/ErrorDetail 字段也会影响没有 `..Default::default()` 的 struct literal。

所以：选择新私有编号及服务端同步属于提供方/本地服务协调；新增公开 RPC 的 schema/router/converter、健康身份校验、attempts 保真和测试 fixture 更新属于纯客户端可安全适配。未经编号/部署决定，不应修改 61/62。

## 保留旧 LocalBridgeV1 wire 的两种实现方案

### 方案一：Local 冻结合同与 External 新版分别生成（推荐）

把两份 wire 合同明确视为不同 transport profile：

- `LocalBridgeV1` 从一份冻结的 local proto 生成。它保留旧公开 0..=60、私有 `ChainBatch=61`、`BenchmarkBars=62`、两个私有 RPC，以及本地 `QueryResponse.source=11`。
- `ExternalV1` 逐字从新版公开 bundle 生成。它只承认公开 `CurrentAuctionObservations=61`、`EconomicReleaseObservations=62`、`EconomicReleaseSchedule=63` 和新版 Health/Listener/ErrorDetail。
- 两套生成类型放在不同 Rust module；业务层使用独立的 typed method/domain operation，不把任一生成 `Operation` 暴露为跨 transport 的统一身份。进入 transport 时显式映射，出来时转换为共享的 domain envelope/evidence。
- 两份 proto 即使保持相同 wire package/method path，也必须进入不同生成输出目录/module，不能让 `include_proto!` 文件互相覆盖。local 生成只代表既有私有部署合同，不得写入或改名为新版公开 descriptor。

最小 domain 边界至少需要区分：

```text
LocalMethod::ChainBatch       <-> local RPC ChainBatch, raw response/error op 61
LocalMethod::BenchmarkBars    <-> local RPC BenchmarkBars, raw response/error op 62
ExternalMethod::CurrentAuctionObservations <-> external RPC, raw op 61
ExternalMethod::EconomicReleaseObservations <-> external RPC, raw op 62
ExternalMethod::EconomicReleaseSchedule     <-> external RPC, raw op 63
```

共享 `QueryRequest` 的 wire shape 虽兼容，也不应因此复用枚举解释。local `QueryResponse.source=11` 可转换到 domain `QueryResult.source`；ExternalV1 的 acquisition authority 应由已认证连接上下文写入 domain evidence，而不是修改公开生成的 `QueryResponse` 来假装公开 Proto 含有 field 11。

优点：descriptor 与每个 transport 的真实语义一致；Capabilities、响应、ErrorDetail 和重放都没有 raw 61/62 二义性；旧 Local 服务不必改号或重启。缺点：生成配置、client wrapper、共享 message 转换和测试 fixture 的改动较大；同名 Protobuf package 的双输出需要构建脚本显式隔离。

这是保留旧 Local wire 的最小**语义安全**方案。它不要求服务端发新号，也不会把 local raw 61 描述成上游竞价。

### 方案二：单 External descriptor + 按 transport/method 解释 raw operation（较小但脆弱）

也可以只生成新版公开 enum，另外只追加私有 RPC method（不追加私有 enum 值），然后让 local client 在调用 `ChainBatch`/`BenchmarkBars` 后直接读取 `QueryResponse.operation` 原始 `i32`：Local method 分别接受 raw 61/62，External method 分别接受公开 61/62/63。普通请求不带 operation 数字，所以它不会主动把旧号发给新版 server。

但这个方案存在结构性风险：单一 descriptor 明确把 61/62 命名为公开 Auction/Economic；同一生成 `QueryResponse` 承载 local 响应时，任何 `Operation::try_from`、生成 accessor、通用日志、Capabilities 解析、ErrorDetail 恢复或后续持久化都可能把 local 61/62 错标为公开 operation。要使它勉强安全，必须同时做到：

1. 所有校验改为 `(ContractProfile, RpcMethod, raw_i32)`，禁止先转统一生成 enum；
2. local Capabilities 的 raw 61/62 只在 local profile 下解释，不能进入 External ready-operation set；
3. ordinary ErrorDetail、trailer、BenchmarkErrorDetail 和持久化恢复 API 全部携带 profile+method；
4. 任何保存 raw Protobuf bytes 的记录都同时保存 contract profile、descriptor SHA 和 method identity；无标签旧记录 fail-closed；
5. 通用 `method_name(Operation)`、`schema_for(Operation)` 和 `implemented_operations()` 改为 domain method 表，避免公开 enum 代表私有 RPC；
6. 测试禁止 local raw 61 被渲染、计数或判定为 `CurrentAuctionObservations`，也禁止 external raw 61 被接受为 `ChainBatch`。

这已经超出一个局部 response shim；漏掉任一控制面或恢复入口就会产生静默语义错标。因此它只在短期桥接且能审计全部入口时可选，不应作为长期统一 descriptor 设计。

### 比较与建议

| 维度 | 双生成/明确转换 | 单 descriptor/raw 适配 |
| --- | --- | --- |
| 旧 Local 61/62 | 原样保留 | 原样保留，但必须绕过生成 enum 语义 |
| External 新 61/62/63 | 公开 descriptor 原样表达 | 公开 descriptor 原样表达 |
| descriptor 真实性 | 两边都真实 | local 响应与 descriptor 枚举名冲突 |
| Capabilities/ErrorDetail | 类型级分离 | 每个入口都必须携带 profile+method |
| 持久化/恢复 | 按生成类型及 profile 天然分离 | 必须补标签和迁移/拒绝旧无标签记录 |
| 初始文件改动 | 较大 | 看似较小，实际横切面广 |
| 静默错标风险 | 低 | 高 |

建议选择方案一；在提供方提交旧 Local descriptor/operation 证据前，只能先搭类型与测试边界，不能宣称 wire 兼容完成。

## Local 服务、能力、错误与持久化的联动清单

### 生成与服务实现

- W `build.rs:37-44` 当前同时 `.build_server(true)` 与 `.build_client(true)`，一个合并 descriptor 生成全部类型和 trait。
- 仓内没有生产 `MarketDataService`/`SystemService` 实现或 `grpc_market_server` binary；检出的 service impl 都是 `grpc_client` 测试 loopback/fixture。故不能从 W 证明真实 Local 服务使用哪个 descriptor、是否仍回 raw 61/62，或能否同步部署。
- 生成 trait 变化会联动 `client.rs` 以及五个 loopback fixture；这是测试编译面，不是生产服务已更新证据。
- 提供方必须给出 Local 服务实际 descriptor SHA、构建身份、`ChainBatch`/`BenchmarkBars` 成功响应 operation、普通 ErrorDetail 和 BenchmarkErrorDetail 的真实编号，再决定兼容路径。

### Capabilities

- `Capability.operation` 也是 raw `i32`。External readiness 当前按 `operation as i32` 直接比较并缓存（`data_gateway/grpc_source.rs:846-910,2635-2639,2953-3040`）；新版公开 61/62/63 可沿 External 类型安全比较。
- 若 Local Capabilities 暴露 61/62，必须在 Local profile 下解码为私有 method；不能交给公开 `Operation::try_from`。`grpc_local_readiness_probe.rs:79-95,136-149` 的比较函数也需明确它使用哪份合同。
- `grpc_bundle_probe.rs:14-18,51-68` 是 External allowlist/capability 视图；新增业务接口时必须用公开类型和真实 schema 接线，不能只把名字加进 allowlist 就当完成。

### QueryResponse 与错误回执

- 正常响应：`grpc_client/envelope.rs:75-94` 当前只按统一 enum 数字比较；Local legacy 兼容必须发生在此函数之前或由 profile-specific parser 取代。
- 普通错误：`grpc_client/errors.rs:297-304,338-390` 会先把 raw operation 转成统一生成 enum，并在恢复时复用相同逻辑（`:455-499`）。双生成方案应提供 local/external 两个 decoder 后再转 domain error；raw 适配方案必须给这些 API 增加 profile+method。
- Benchmark 专用错误：`data_gateway/grpc_source.rs:480-505` 直接要求 `BenchmarkErrorDetail.error.operation == Operation::BenchmarkBars as i32`；保留旧 Local 服务时必须继续验证 raw 62，但验证依据应是 Local contract，而不是新版公开 enum。
- standard details 与 `magic-error-detail-bin` trailer 的一致性检查不能因为兼容而放宽；只允许在两边字节一致后按对应 profile 解释。

### 持久化与恢复

- `grpc_client/external_control_attempt.rs` 会保存并恢复 Health/Capabilities 原始 Protobuf bytes；`push_foundation/intent_store/chain_post_close_macro_codec.rs` 解码这些 External control 响应。它们应固定使用 External 新版生成类型，并把 descriptor/build identity 纳入恢复资格。
- `grpc_client/macro_attempt.rs`、`board_attempt.rs` 及其 durable 调用方也保存请求/响应或 status detail bytes，并在恢复时带有 profile/请求身份。任何通用恢复 helper 都不得丢失 profile 后再解释 raw operation。
- 当前 ChainBatch 的持久结果是转换后的 `VisibleChainBatch` domain/JSON，未发现把其原始 `QueryResponse` bytes 作为恢复源；这降低迁移面，但首次 wire 解析仍必须按 Local 合同验证 raw 61。
- BenchmarkBars 把专用 status detail 先分类为 `BenchmarkGrpcFailure`/审计 receipt，再进入客户端审计；未发现生产路径长期保存原始 `BenchmarkErrorDetail` bytes，但 `audit_state` 的可信分类依赖先按 Local raw 62 校验，不能改成公开 Economic operation。
- 对任何历史上未记录 contract profile/descriptor SHA 的 raw 61/62 Protobuf 证据，迁移时应 fail-closed 或由已证明的存储上下文一次性标注；不得仅凭数字猜 Local/External。

## 原 GD-001～015 在新版下的状态

“接口已有候选”不等于下游已接线，更不等于生产已验证。

| ID | 新版影响 | 当前判定 |
| --- | --- | --- |
| GD-001 | Health 新增 build identity 与运行计数，可证明实际制品和处理状态 | **证据接口增强；运行问题未解决。** W 尚未比较 identity，本轮也未发 RPC |
| GD-002 | 新增 CurrentAuctionObservations，记录可提供 `auction_volume_ratio` | **上游窄字段候选已发布；下游仍未接。** 值可空，且无 trading date/source_at，不能直接替换严格日期竞价合同 |
| GD-003 | 同一新 RPC 可补集合竞价量比；MoneyFlows 公开合同仍存在 | **部分接口条件改善，未闭环。** `main_net_yi`、跨批组合和原消费者接线不由新 RPC 自动解决 |
| GD-004 | 新 Proto 未改变 Consensus payload；文档仅强化冲突错误字段 | **仍需下游转换/合同证据。** recent reports/date/target price 丢失问题未解决 |
| GD-005 | 新版未增加 EPS 报告期/预测年度/累计单季/issuer 的 Proto 字段 | **未解决。** 需 payload 合同和消费者口径证据 |
| GD-006 | 市场服务仍明确不提供账户、资产、持仓或委托接口 | **不属于 bundle 修复。** 仍需账户来源与业务门修复 |
| GD-007 | ProviderTopN 没有本次 Proto 变化 | **未解决。** 远端 date-only 与本地 limit/filter 分层仍需确认 |
| GD-008 | T0Evidence 仍为 v2、要求精确 requested_at；现有保护不应删除 | **现状保持；仍待真实批次验收。** 不是新版需放宽/收紧的缺陷 |
| GD-009 | 新增 EconomicReleaseObservations(limit/country) 与 Schedule(start/end/limit) | **接口层已有明确替代候选；W 未适配。** 旧 EconomicCalendar 仍 fail-closed，不能无语义选择地互换 |
| GD-010 | 新增 gRPC ordered-route `provider_attempts` | **不解决搜索聚合器丢失败。** W 还会丢新版 gRPC attempts，需另补保真 |
| GD-011 | 无账户快照、quote lineage 或时效合同变化 | **未解决，纯下游问题** |
| GD-012 | 无板块 batch lineage 合同变化 | **未解决，纯下游投影问题** |
| GD-013 | 无双榜/新闻归因 lineage 合同变化 | **未解决，纯下游投影问题** |
| GD-014 | Announcements 没有新增 date/limit 请求合同 | **未解决。** 不得把新经济 RPC 外推到公告 |
| GD-015 | BlockTrades 没有本次接口变化 | **未解决。** 审计 hash、集合/日期校验、成交时间与单位仍属下游及服务端字段证据 |

端到端“已解决”的原问题为 **0 项**。接口层新增了 GD-002/003 的窄竞价量比候选和 GD-009 的两个明确经济数据合同；它们都需要 W 适配及真实批次验收。GD-001 获得更强的诊断证据字段，但没有获得本轮运行响应。

## 建议的最小实施拆分

### A. 可先做、但仍需避开当前 Rust 作者文件

1. **生成兼容面**：在编号决策后修改 `build.rs`；同步 `grpc_contract/ops.rs`、`grpc_contract/schema.rs`、`grpc_client/client.rs`。
2. **公开 ExternalV1 合同**：在 `grpc_client/external_v1.rs` 增加三个严格请求构造器，逐字段校验 instrument/stage、limit/country、start/end/limit；不要复用旧 `EconomicCalendar {}`。
3. **下游 typed adapter**：在 `data_gateway` 新建或扩展竞价 observations 与经济 observations/schedule 类型及 converter；保留 Provider、批次、逐条 evidence、nullable 时间/量比和 verified-empty 的窄语义。
4. **错误证据**：在 `grpc_client/errors.rs` 保存有界 `provider_attempts`；unknown 值 fail-closed，不从自由文本推断。
5. **控制面**：对 ExternalV1 Health 比较 bundle/descriptor identity，并把 observability/listener counters 仅作为观测输出，不作为 admission 或市场数据 evidence。

### B. 必须先协调的项目

1. `ChainBatch`/`BenchmarkBars` 的最终私有编号或独立 service/package。
2. 若旧本地服务不能同步部署，是否接受 `LocalBridgeV1` method-specific legacy 61/62 适配；需要同时覆盖成功响应和 ErrorDetail/BenchmarkErrorDetail。
3. CurrentAuctionObservations 是否满足 P-02 对交易日和新鲜度的业务门；公开合同明确没有 trading date/source_at，客户端不得自行补造。
4. GD-009 消费者究竟要“已发布观测”还是“官方未来日程”；两者 provider、时间精度和 verified-empty 含义不同。

## 受影响文件清单

### 核心编译/协议面

| 文件 | 证据/最小变更 |
| --- | --- |
| W `build.rs:49-127` | 61/62 重号根因；编号/拆服务决定后更新 append-only 合并规则并增加重复数字检测 |
| W `src/grpc_contract/ops.rs:1-3,7-73,76-152` | 补 61～63 名称与覆盖测试，移除旧 0..=62 假设；私有 op 按最终方案隔离 |
| W `src/grpc_contract/schema.rs:209-219` | 保留两项私有 schema；增加三个公开 schema 或把 ExternalV1 schema 独立管理 |
| W `src/grpc_client/client.rs:373-435` | 三个新增 RPC 的 method router；profile 必须阻止私有 op 进入 ExternalV1 |
| W `src/grpc_client/envelope.rs:34-57,75-94` | 若保留旧本地服务，在 method/profile 边界做窄 legacy response normalize；不得全局把公开 61/62 重解释 |
| W `src/grpc_client/external_v1.rs:25-116` | 新增三份公开请求合同与严格字段验证 |
| W `src/grpc_client/errors.rs:338-390` | 保存/校验 `provider_attempts`；兼容旧 detail 缺字段 |

### Gateway/消费面

| 文件 | 证据/最小变更 |
| --- | --- |
| W `src/data_gateway/grpc_source.rs:3017-3039` | 外部控制门比较 build identity；新增公开 RPC 查询入口 |
| W `src/data_gateway/grpc_source.rs:3333-3340` | 当前 EconomicCalendar 仍发 `{}`；保留 fail-closed，不能原地冒充新 operation |
| W `src/data_gateway/grpc_source.rs:3949-4010` | ChainBatch 真实 local 路径与 legacy 61 响应兼容点 |
| W `src/data_gateway/grpc_source.rs:480-505,2891-2950,3222-3226` | BenchmarkBars 真实 local 路径及成功/错误编号兼容点 |
| W `src/data_gateway/grpc_source/convert.rs` | 新增三类严格 schema/version/字段/evidence converter |
| W `src/data_gateway/economic_calendar.rs:10-75` | 现类型语义是 Jin10 release observation，可复用字段但必须改为新 operation 且保留窄窗口含义；FRED schedule 应使用独立类型 |
| W `src/data_gateway/mod.rs` | 导出新 typed gateway/record |

### 生成 trait 与 struct literal 测试面

新增三个 RPC 后至少这些 `MarketDataService` 测试实现需补方法：

- `src/grpc_client/client.rs`
- `src/grpc_client/board_loopback_fixture.rs`
- `src/grpc_client/dragon_tiger_attempt_tests.rs`
- `src/grpc_client/macro_loopback_fixture.rs`
- `src/grpc_client/macro_full_loopback_fixture.rs`
- `src/grpc_client/external_control_loopback_fixture.rs`

新增 Health 字段会影响 `grpc_client/external_control_attempt_tests.rs`、`external_mtls_attempt_tests.rs`、`external_control_loopback_fixture.rs`、`client.rs` 及若干 `push_foundation/intent_store/*macro_control*_tests.rs` 中的完整 `HealthResponse` literal。新增 ErrorDetail 字段会影响 `grpc_client/errors.rs`、`bin/grpc_bundle_probe.rs`、`data_gateway/grpc_source.rs` 中没有结构体更新语法的 literal。应优先使用明确的新字段或 `..Default::default()`，不要为通过编译填造观测值。

## 最小测试矩阵

1. **Proto merge 静态测试**：新版公开 0..=63 每个数字唯一；私有声明不得占用公开数字；RPC 名和 Operation 一一映射。
2. **profile 隔离**：Local ChainBatch/BenchmarkBars 只能走 local channel；ExternalV1 在发送前拒绝它们。公开 61/62 只能路由到对应公开 RPC。
3. **旧本地服务兼容（若采用）**：按方法分别接受 legacy response 61/62；相同 raw 值在 ExternalV1 仍解释为公开 operation；任意跨方法错号继续 fail-closed。成功响应、普通 ErrorDetail 和 BenchmarkErrorDetail 都覆盖。
4. **新增公开请求**：stage 非法、重复/空 instrument、limit 越界、country 类型错误、日期倒置/跨度超限全部在发送前拒绝；请求 schema/version/provider 精确匹配文档。
5. **竞价转换**：量比 null/有限正值、未成交价、负 `auction_unmatched`、source_at null、返回缺失/额外/重复证券；不得补交易日或买卖方向。
6. **经济观测**：完整空窗口为 VerifiedEmpty，但不能标成某日/未来日历为空；逐条 released/source 时间一致。
7. **经济日程**：date-only 不转午夜或 source_at；范围、排序、limit、重复 release ID 明确验证。
8. **错误 attempts**：0/1/16 项、超过 16、ordinal 乱序/重复、unknown provider/outcome/reason、terminal 与 retryable 组合、standard/trailer 相同或冲突、持久化恢复。
9. **Health identity**：匹配、contract mismatch、binary/source 缺失、identity_error；live+ready 但 identity 不匹配必须拒绝升级为已认证制品。
10. **旧客户端兼容**：服务端新增 append-only 字段时旧逻辑继续可解码；缺 observability/build_identity/provider_attempts 仍按明确 legacy 状态处理。

## 未验证事项

- 没有运行代码生成或 Cargo，因此本文用静态 Proto/Rust 证据指出预期编译面，没有把它表述为实际编译日志。
- 没有连接本地或 ExternalV1 服务，没有 Health、Capabilities、Listener、业务响应或错误 trailer 的真实样本。
- 没有证明当前运行二进制对应新版 bundle 的 source commit、descriptor 或 binary SHA。
- 没有证明 Hithink Key、FRED 身份、Jin10 route 或任一 runtime capability 可用。
- 没有验证旧本地服务真实返回的 ChainBatch/BenchmarkBars operation 数字及 ErrorDetail；61/62 来自当前 W 合同与客户端严格校验，部署制品仍需提供方证明。
- 没有验证 CurrentAuctionObservations 能满足 P-02 的交易日/新鲜度门；公开合同本身明确缺少这些字段。
- 没有验证 EconomicReleaseObservations/Schedule 的真实记录、分页、空结果或时间语义。
- 本报告不授权修改/重启服务、不批准私有编号，也不将任何 GD 项标记为生产完成。

## 本次读取的 W 核心快照

| 文件 | SHA-256 |
| --- | --- |
| `build.rs` | `2a24b233bc7c1ec19f64fe0a6e7ba6163b454c3666768bca3ec4d85772131587` |
| `src/grpc_contract/ops.rs` | `452746f2161ca1288859e03cea44cdf77cef953b822264216873879ca3d87503` |
| `src/grpc_contract/schema.rs` | `6879a721eb57c85e431731d8a928ff287c08311ca7bb3c8e3d51f927b45b65e0` |
| `src/grpc_client/client.rs` | `e6b629461310d45dce5dd394e0707742e462d3f9d4183d135d7ec10a02b7124b` |
| `src/grpc_client/errors.rs` | `1958e0464a0dec061e62528982615840ac3cb886e59e97c9074299b4179a7401` |
| `src/data_gateway/grpc_source.rs` | `c7a131d43285cd936e1cbdb58e5d1ce960a54d94cbcfc3e3d1df3dca0485ceba` |
| `src/data_gateway/grpc_source/convert.rs` | `d1bef9d1ed38d5c97e9a59577f9d48bbee84a44a0bdab1bd2e282e51ee82b375` |
| `src/data_gateway/economic_calendar.rs` | `c9f4dbb658db47ce93f3f81d465b2e5b2313772b359bac0dad30280d73198e6e` |
| `src/data_gateway/mod.rs` | `767521927ef9cd64d748f363c4d581787ecd1e7b9880a2d470f5d220d904833c` |

共享开发树仍可能变化；实施时必须重新核对 SHA 和行号，尤其不要覆盖并行账户推送修改。
