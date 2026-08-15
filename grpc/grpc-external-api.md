# Magic Market gRPC 对接文档

## 1. 当前状态

| 项目 | 状态 |
| --- | --- |
| Protobuf v1 合同 | 已建立，可生成客户端 |
| 54 个只读数据族 RPC | 已进入 v1 Proto |
| 能力与健康接口 | 已进入 v1 Proto |
| TDX 异动订阅、重放、Agent 流 | 已进入 v1 Proto |
| gRPC Server | 开发中，尚未提供生产地址 |
| TDX 数据/异动正式准入 | `false`，当前只能作为诊断/影子事件 |

另一个项目现在可以根据 Proto 生成客户端、完成接口封装和假服务测试，但在服务端
发布地址、TLS 证书和认证令牌交付前，不应把联调状态标记为生产可用。

## 2. 合同源文件

唯一合同源：

```text
crates/magic-market-grpc-contracts/proto/magic/market/v1/market.proto
```

Protobuf package：`magic.market.v1`，当前协议版本：`1`。

调用方不得复制并自行修改 Proto。升级时以仓库内文件和 descriptor set 为准。

## 3. 网络地址

本机诊断：

```text
http://127.0.0.1:<operator-port>
```

远程部署：

```text
https://<server-name>:<operator-port>
```

- 服务不约定固定端口，端口由部署方显式提供。
- 默认只允许 loopback。
- 非 loopback 必须启用 TLS 和认证。
- 客户端不得直接访问 TDX 的 `127.0.0.1:17709`。

## 4. 认证

业务客户端通过 gRPC metadata 发送：

```text
authorization: Bearer <token>
```

远程生产环境优先使用 mTLS；Bearer Token 是应用身份，不得放进 Protobuf 请求体、
URL、日志或错误信息。服务端发布时会同时交付服务 DNS、TLS CA/证书链、客户端身份
以及服务端配置的消息、并发和流上限。

## 5. 通用请求合同

所有查询 RPC 使用 `QueryRequest`：

```proto
message QueryRequest {
  RequestContext context = 1;
  string preferred_provider = 2;
  CanonicalPayload payload = 3;
}
```

### RequestContext

```text
protocol_version = 1
request_id       = 调用方生成的非空唯一请求 ID
```

同一业务重试应保留原 `request_id`，并由调用方另外记录 retry attempt。

### preferred_provider

- 空字符串：由服务端正式 Composition/Router 选择来源。
- 非空：必须精确匹配服务端已登记 Provider。
- 不能填写 URL、IP、代理、任意 Provider 名称或动态插件名。

建议普通调用保持为空。

### CanonicalPayload

```text
schema         = 方法登记的请求 schema 名称
schema_version = 正整数，当前为 1
content_type   = application/json; charset=utf-8
data           = UTF-8 JSON 字节
```

第一版 gRPC 使用 Protobuf 作为传输和服务合同，现有 Rust Serde JSON 作为每个数据族的
规范化业务 payload。每个方法的 schema 名称和 JSON 字段在服务端 Provider 接入时
单独冻结；调用方遇到未知 schema/version 必须停止解析，不能忽略或猜字段。

### 请求 params (M1 起)

`payload.data` 是 **params JSON 对象**（UTF-8 JSON 对象，不是数组）：

- `{}` = 全默认 = 与 library 模式生产行为一致；
- 显式字段才改变行为（服务端不发明调用方未声明的默认）；
- 字段解析失败 → `Status::invalid_argument`（400 语义，错误消息含字段名）。

参数表（op → 字段 → 默认）：

| 字段 | 含义 | 默认 |
|------|------|------|
| codes | 逐代码 op 的代码列表（字符串数组） | STOCK_LIST env (watchlist) |
| days | HistoricalBars 天数 | 120 |
| count | TechnicalBars 15min K 线条数 | 48 |
| date | UpperLimitPoolReview / ProviderTopNRankings 日期 YYYY-MM-DD | 今天 |
| from_days | InstrumentNews 回溯天数 | 30 |
| interval / limit | FundFlowSeries 周期 (day1/minute1) / 条数 | "day1" / 20 |
| kind | FinancialStatements (balance/income/cash_flow) | 必填，缺失报错 |
| code / window_start / window_end | CorporateActions 代码与窗口 (YYYY-MM-DD) | 必填，缺失报错 |
| query | SemanticSearch 检索词 | 必填，缺失报错 |
| observed_at | T0Evidence 观测时刻 (RFC3339) | 服务端现在时刻 |

实现：`src/grpc_contract/params.rs`（服务端与客户端共享契约，默认值表即该模块文档）。

## 6. 通用响应合同

`QueryResponse` 包含：

```text
request_id
operation
admission
selected_provider
batch_id
complete
observed_at
source_at
records[]
```

调用规则：

- `admission=ADMITTED` 才能作为生产数据使用。
- `complete=false` 不能被当作成功完整批次。
- `source_at` 为空表示来源没有提供可信源时间；不能用 `observed_at` 代替。
- `records[]` 中每项都有独立 schema/version/content-type。
- `batch_id`、Provider、单位和来源证据必须原样保存。

## 7. 服务与方法

### SystemService

```text
GetCapabilities
GetHealth
```

启动后应先调用 `GetCapabilities`。RPC 存在不等于对应能力已经准入；每个能力同时返回
repository admission、runtime availability、精确范围和 blocker。

### MarketDataService

```text
HistoricalBars             MinuteData
RealtimeQuotes             MoneyFlows
OrderBooks                 Auctions
Trades                     SecurityMetadata
GlobalIndices              ForeignExchange
EconomicCalendar           FuturesDelivery
ReferenceRates             OfficialFxFixings
EconomicSeries             CompanyFilings
GlobalNews                 Announcements
MarketAnnouncements        InvestorQuestions
PolicyDocuments            SecurityProfiles
FinancialStatements        MarketStatistics
TechnicalBars              CorporateActions
BoardDirectory             BoardConstituents
BoardMemberships           ResearchReports
ResearchDocuments          Consensus
TargetPrices               SemanticSearch
FundFlowSeries             BoardFlows
MarginData                 BlockTrades
HolderCounts               LockupEvents
DividendPlans              PostCloseFlows
NorthboundDaily            LimitPools
StrongStockReasons         DragonTiger
MarketDragonTiger          DragonTigerDiscovery
MarketRankings             MarketBreadth
Popularity                 ConceptHits
OptionData                 ProviderTopNRankings
```

所有方法都是只读 unary RPC。没有账户、资产、持仓、委托、撤单或成交写接口。

## 8. TDX 价格异动订阅

业务消费方调用 `MarketEventService.Subscribe`：

```proto
message SubscribeRequest {
  RequestContext context = 1;
  EventFilter filter = 2;
  EventCursor after = 3;
}
```

`EventFilter`：

- `instruments` 为空表示服务端授权范围内的全部标的；
- 非空时使用服务端发布的规范 instrument ID；
- `event_kinds` 可选择价格、成交量、成交额、状态和 reset 类事件；
- 未知值不能被当作通配符。

返回 `stream MarketEventEnvelope`：

```text
event_id
cursor.generation
cursor.sequence
event_kind
provider
instrument
observed_at
source_at
admission
payload
```

消费方必须持久化最后成功处理的 `generation + sequence`。generation 改变表示 TDX
重启、终端替换或服务明确重建连续性，不能把新旧 generation 拼成连续行情。

### 断线恢复

调用 `MarketEventService.Replay` 并传入最后已处理 cursor。重放是有界、同 generation、
best-effort：

- 返回成功：按 sequence 顺序处理；
- `OUT_OF_RANGE`：cursor 已早于重放窗口，调用方记录明确 gap；
- `FAILED_PRECONDITION`：generation 不匹配或连续性已重置；
- 不得把重放描述为 exactly-once 或 at-least-once。

### TDX 当前准入状态

目前 TDX `price/volume/amount` 和本地异动均为 `UNADMITTED`。联调时仍可能收到
`admission=UNADMITTED` 的影子事件，另一个项目必须显式展示/隔离，不能用于生产告警
或交易决策。

## 9. TDX Agent 接口

`TdxAgentService.OpenStream` 只供同仓库 Windows Agent 使用，普通业务系统不要调用。

```text
Windows TDX Agent --client stream--> gRPC Server
Windows TDX Agent <--server commands-- gRPC Server
```

第一条消息必须是 `AgentHello`，后续只能发送有序 Event 或 Heartbeat。服务端返回 Ack
或 Stop。协议没有下单、撤单或账户命令。

## 10. gRPC 错误处理

| gRPC code | 调用方行为 |
| --- | --- |
| `INVALID_ARGUMENT` | 修正请求/schema/cursor，不自动重试 |
| `UNAUTHENTICATED` | 刷新或更换凭据 |
| `PERMISSION_DENIED` | 停止该能力调用并联系授权方 |
| `UNIMPLEMENTED` | 能力未准入或不支持，不重试 |
| `RESOURCE_EXHAUSTED` | 按服务端策略退避；流消费者需记录 gap |
| `DEADLINE_EXCEEDED` | 有界退避重试，保留原 request_id |
| `UNAVAILABLE` | 有界指数退避，重新检查 health/capabilities |
| `FAILED_PRECONDITION` | 数据完整性/连续性失败，不能当空成功 |
| `INTERNAL` | 记录 request_id，停止无界重试 |

服务端可能把安全的 `ErrorDetail` 编码在 Status details 中：request ID、operation、
Provider、reason code 和 retryable；不得依赖自然语言 message 做程序分支。

## 11. 客户端代码生成

### Python

```bash
python -m grpc_tools.protoc \
  -I crates/magic-market-grpc-contracts/proto \
  --python_out generated \
  --grpc_python_out generated \
  crates/magic-market-grpc-contracts/proto/magic/market/v1/market.proto
```

### Go

```bash
protoc \
  -I crates/magic-market-grpc-contracts/proto \
  --go_out generated --go_opt=paths=source_relative \
  --go-grpc_out generated --go-grpc_opt=paths=source_relative \
  crates/magic-market-grpc-contracts/proto/magic/market/v1/market.proto
```

Go 项目正式接入前可在自己的 Proto 镜像中补 `go_package` 映射，但不得修改字段号、
枚举值或 service/method 名称。

### Rust

使用 `tonic-prost-build` 编译同一 Proto，或直接依赖同版本
`magic-market-grpc-contracts` crate。禁止从服务端内部 crate 引用业务实现。

## 12. 联调检查表

- [ ] Proto 文件摘要与服务端发布版本一致。
- [ ] `protocol_version=1`，request_id 非空且可检索。
- [ ] 启动先调用 GetHealth 和 GetCapabilities。
- [ ] 远程连接验证 TLS hostname 和 CA。
- [ ] Authorization 只在 metadata 中注入。
- [ ] 为 unary 和 stream 分别设置客户端 deadline/keepalive。
- [ ] 不把 UNADMITTED、partial、缺 source_at 当作生产成功。
- [ ] 持久化 TDX generation/sequence，并处理 gap/reset。
- [ ] 对 RESOURCE_EXHAUSTED/UNAVAILABLE 使用有界退避。
- [ ] 日志不输出 Token、完整敏感 payload 或上游凭据。

## 13. 服务端发布时需要交付给对接方

1. `market.proto` 和 descriptor set 摘要；
2. 服务地址、TLS CA、认证材料；
3. 服务端消息/并发/流/重放限制；
4. 已准入 capability 快照与精确 scope；
5. 每个已启用方法的 canonical request/record schema fixture；
6. TDX 是否仅影子模式及其 admission 状态；
7. 版本升级和字段废弃通知周期。
