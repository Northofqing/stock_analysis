# Magic Market gRPC 对接文档

## 1. 当前状态

| 项目 | 状态 |
| --- | --- |
| Protobuf v1 合同 | 已建立，可生成客户端 |
| 54 个只读数据族 RPC | 已进入 v1 Proto |
| 能力与健康接口 | 已进入 v1 Proto |
| TDX 动态监控列表、异动订阅、重放、Agent 流 | 已进入 v1 Proto |
| gRPC Server | 已实现并在当前 Windows 工作站运行受限联调实例 |
| Unary Provider composition | 54 个操作精确登记；46 个正式 handler，6 个 opt-in 诊断 handler，2 个无数据操作 fail-before-I/O |
| TDX 数据/异动正式准入 | `false`，当前只能作为诊断/影子事件 |

另一个项目现在可以根据 Proto 生成客户端并连接当前受限联调实例。实例地址、证书和
Token 仍属于部署材料而不是稳定公共地址；迁移主机、IP 或证书后必须重新交付连接包。

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
- 非 loopback 必须启用双向 TLS（mTLS）和 Bearer 认证。
- 客户端不得直接访问 TDX 的 `127.0.0.1:17709`。

当前工作站联调实例为 `https://10.211.55.3:50051`，TLS server name 是
`magic-market.local`；仅允许配置的局域网网段并强制 mTLS + Bearer。客户端材料在
服务端本机 `target/runtime/client-bundle/`，不得提交到 Git 或通过公开渠道传输。

## 4. 认证

业务客户端通过 gRPC metadata 发送：

```text
authorization: Bearer <token>
```

远程环境强制使用 mTLS；Bearer Token 是额外的应用身份，不得放进 Protobuf 请求体、
URL、日志或错误信息。服务端发布时会同时交付服务 DNS、TLS CA/证书链、客户端身份
以及服务端配置的消息、并发和流上限。

## 5. 通用请求合同

所有查询 RPC 使用 `QueryRequest`：

```proto
message QueryRequest {
  RequestContext context = 1;
  string preferred_provider = 2;
  CanonicalPayload payload = 3;
  bool allow_unadmitted = 4;
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

### allow_unadmitted

- 默认 `false`：未准入能力继续在 Provider I/O 前返回 `UNIMPLEMENTED`；
- `true`：只允许执行服务端预先登记的诊断 handler，不能注入 URL/方法/Provider；
- 诊断响应必定是 `admission=UNADMITTED`、`complete=false`，并返回
  `diagnostic_blocker`；
- 该开关只用于开发联调和证据采集，不能用于生产告警或交易决策。

服务端把 `--provider-timeout-ms` 与 `--blocking-deadline-ms` 分开配置。前者约束每次
Provider 网络调用，后者约束包含分页、解析和规范化在内的完整阻塞任务；前者不得大于
后者。这样全市场诊断可以拥有更长的有界总预算，而不会放宽任何 Provider 的单请求
HTTP 超时。

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
diagnostic_blocker
```

调用规则：

- `admission=ADMITTED` 才能作为生产数据使用。
- `complete=false` 不能被当作成功完整批次。
- `source_at` 为空表示来源没有提供可信源时间；不能用 `observed_at` 代替。
- `records[]` 中每项都有独立 schema/version/content-type。
- `diagnostic_blocker` 非空表示本次是显式诊断读取；即使 records 非空，也不能视为准入。
- `batch_id`、Provider、单位和来源证据必须原样保存。

## 7. 服务与方法

### SystemService

```text
GetCapabilities
GetHealth
```

启动后应先调用 `GetCapabilities`。RPC 存在不等于对应能力已经准入；每个能力同时返回
repository admission、runtime availability、diagnostic availability、精确范围和 blocker。

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

### 动态指定监控标的

`Subscribe.filter.instruments` 只过滤已经采集的消息，不改变 TDX 实际监控范围。控制方先
调用 `MarketEventService.SetWatchlist`，每次传入完整的新列表：

```proto
message SetWatchlistRequest {
  RequestContext context = 1;
  repeated string instruments = 2;
}
```

只接受非空、无重复的 `EQUITY:SH:600396`、`EQUITY:SZ:000001` 或
`EQUITY:BJ:430001` 形式。列表长度不能超过当前 Agent 在
`GetListenerStatus.maximum_watchlist_instruments` 中公布的上限。例如 JSON 请求：

```json
{
  "context": { "protocolVersion": 1, "requestId": "watchlist-20260815-1" },
  "instruments": ["EQUITY:SH:600396", "EQUITY:SZ:000001"]
}
```

成功响应的 `state` 为 `restarting` 或 `unchanged`。`restarting` 只表示命令已进入当前
Agent 的有界命令队列；调用方应轮询 `GetListenerStatus`，直到
`desired_watchlist_revision == applied_watchlist_revision` 且两份列表完全相等。列表改变会
重启固定 sibling monitor、创建新 generation 并清空旧窗口/重放，不能把旧 cursor 用于
新列表。没有活动 Agent 时返回 `UNAVAILABLE`，超上限或格式错误返回
`INVALID_ARGUMENT`。

动态控制是全局全量替换，不是追加，也不会按订阅者自动合并。多个控制方需要在调用方
侧协调；普通消费者只使用 `Subscribe.filter`。

动态列表只保存在当前 Server/Agent 运行期内。Server 与 Agent 同时重启后，会重新使用
部署参数文件中的初始 `--watchlist`；如果外部系统需要持久列表，应由它保存期望值，并在
连接恢复后再次调用 `SetWatchlist`，等待 desired/applied 状态一致。

2026-08-15 Windows 真实联调从初始单标的替换为
`EQUITY:SH:600396,EQUITY:SZ:000001`：响应为 `restarting`、desired/applied revision
均为 `1`，Agent 建立了新 generation，随后 Replay 分别收到两只标的各四条 observation。
这证明了实际采集范围发生了变化，不只是订阅端过滤。事件仍保持 `admitted=false`。

### 订阅事件

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

服务端还会再次强制该边界：TDX Agent 若发送 `ADMITTED`，流会以
`FAILED_PRECONDITION` 停止，不能由传输层自行提升 repository admission。

### 已接入的证券资料请求

`SecurityMetadata` 使用以下 canonical schema：

```text
schema = magic.market.security_metadata.request
data   = {"instruments":[{"exchange":"Shanghai","code":"600396","asset_class":"Equity"}]}
```

腾讯来源覆盖 1..=50 个唯一沪深京股票。名称和 ST 标记来自源快照，板块为显式派生；
来源未证明的上市日期、涨跌停规则及规则版本保持 unavailable，因此该调用可能返回
`admission=ADMITTED` 且 `complete=false`。调用方必须保留字段级质量，不能把空字段补成
默认值。

`SecurityProfiles` 使用相同 instruments JSON，schema 为
`magic.market.security_profiles.request`。TDX 公共协议只覆盖 1..=8 个唯一沪深股票，返回
精确名称、可选财务包上市日和唯一 `公司概况` F10 原始行事实；不推断行业、总股本或
流通股本。F10 没有可信源时间，因此 `source_at` 为空。精确范围与实测证据见
[TDX 公共公司资料准入](tdx-public-security-profile.md)。

## 9. TDX Agent 接口

`TdxAgentService.OpenStream` 只供同仓库 Windows Agent 使用，普通业务系统不要调用。

```text
Windows TDX Agent --client stream--> gRPC Server
Windows TDX Agent <--server commands-- gRPC Server
```

第一条消息必须是 `AgentHello`，后续只能发送有序 Event 或 Heartbeat。服务端返回 Ack、
Stop 或严格类型化的只读 watchlist replacement。该命令只能携带 revision 和规范化股票
身份；协议没有 URL、阈值、下单、撤单或账户命令。

Windows Agent 只启动同目录 `magic-market-monitor-server.exe`，并从同目录、最大
64 KiB 的 `magic-market-monitor-server.args.json` 读取 JSON 字符串数组参数；不搜索
`PATH`，也不接受 helper/TDX/17709 地址覆盖。Agent 到远程服务必须提供服务端 CA、
客户端证书和私钥；只有精确 loopback gRPC 地址允许明文。

## 10. 当前实现状态

- Protobuf/descriptor、54 个 unary RPC、health/capabilities、Bearer auth、远程 mTLS、
  blocking 调用隔离均已实现；
- 事件服务已实现严格 generation/sequence、同 generation 有界 replay、过滤和慢消费者
  显式终止；
- TDX Agent 双向流、动态全量 watchlist replacement 和 Windows 固定 sibling monitor
  重启/转发已实现；TDX 事件保持影子模式；
- unary registry 对 54 个操作逐项精确登记：46 个操作绑定 admissions.tsv 范围内的
  Tencent、Sina、Eastmoney、CNInfo、CFETS、FRED、SEC EDGAR、WallstreetCN、Jin10、
  HKEX、THS、State Council、iWencai 或 TDX 公共协议 handler；
- `MoneyFlows`、`FuturesDelivery`、`TechnicalBars`、`FundFlowSeries`、
  `PostCloseFlows`、`MarketRankings` 登记了显式 opt-in 诊断 handler；配置
  `EASTMONEY_API_KEY`（兼容别名 `MX_APIKEY`）后，`Auctions` 和 `MarketBreadth` 也登记
  东财妙想诊断 handler。这 4 个固定模板操作默认即可返回 `UNADMITTED` partial 数据；
  其他诊断仍只有 `allow_unadmitted=true` 才执行；
- 未配置东财妙想 Key 时，`Auctions` 和 `MarketBreadth` 仍在 I/O 前
  `UNIMPLEMENTED`；配置后也只返回源直接给出的部分字段，不用普通 Quote 冒充竞价，
  不把不完整家数统计提升为完整市场宽度；
- `preferred_provider` 非空时必须精确选择已登记来源；空值选择该操作第一个可用登记。
  当前不会在一次请求内部隐藏切源，上游失败会原样形成 typed gRPC error，调用方可根据
  capabilities 和业务路由策略发起有界重试；
- FRED、SEC EDGAR、iWencai 和东财妙想还要求对应运行时环境身份；缺失时 capability 保留
  repository admission、但 `runtime_available=false`，请求会在 I/O 前失败。
- 2026-08-14 当前实例通过 `SemanticSearch` + `preferred_provider=Iwencai` 实测返回
  10 条 `Report` 记录；Key 只从服务进程环境加载，不进入请求、日志或证据。

剩余 8 项不是缺少 gRPC 方法，而是生产数据合同尚未满足。已有字段通过显式诊断模式
读取，缺失字段保留 `null`，但不会改变下表状态：

| 操作 | 当前阻塞原因 |
| --- | --- |
| `MoneyFlows` | 东财妙想可诊断返回日级主力/超大/大/中/小单净额，但方法学和串行稳定性尚未准入；公共 TDX 成交额不能冒充分单资金流 |
| `Auctions` | 东财妙想只证明开盘集合竞价成交量（股）和成交额（元）；撮合价、昨收、未匹配买卖量、量比和 Provider 时间仍为空，不满足完整 BR-035 合同 |
| `FuturesDelivery` | CFFEX 当前 TLS 实测仍在 HTTP 前异常结束，正式能力保持 false |
| `TechnicalBars` | Baidu 技术 K 线尚缺交易日历和公司行动连续性证据 |
| `FundFlowSeries` | 东财妙想可诊断返回日级五档净额，但自然语言结果数量可超过请求数，当前只做有界校验/截断；尚未通过正式 load gate |
| `PostCloseFlows` | 诊断可请求明确的过去日期且逐条校验来源日期；全市场证券源时间仍不一致，不能构成同一盘后原子快照 |
| `MarketRankings` | 诊断只读取首个有界来源页，并返回来源声明总数；不声称完整市场覆盖或源时间原子性 |
| `MarketBreadth` | 东财妙想只证明上涨/下跌/平盘及涨跌停家数；上市总数、覆盖率、来源时间偏差为空，不能提升为完整市场宽度 |

### 诊断请求 schema

所有 payload `schema_version=1`：

| Operation | request schema | record schema |
| --- | --- | --- |
| `TechnicalBars` | `magic.market.technical_bars.request` (`BarsRequest`) | `magic.market.technical_bar` |
| `FundFlowSeries` | `magic.market.fund_flow_series.request` (`FundFlowRequest`) | `magic.market.fund_flow_point` |
| `MoneyFlows` | `magic.market.money_flows.request` (`{"instruments":[...]}`，精确 1 个) | `magic.market.money_flow` |
| `FuturesDelivery` | `magic.market.futures_delivery.request` (`FuturesDeliveryRequest`) | `magic.market.futures_delivery_event` |
| `PostCloseFlows` | `magic.market.post_close_flows.request` (`PostCloseFlowRequest`) | `magic.market.post_close_flow_diagnostic` |
| `MarketRankings` | `magic.market.market_rankings.request` (`{"kind":...,"limit":...}`) | `magic.market.market_ranking_diagnostic_entry` |
| `Auctions` | `magic.market.auctions.request` (`{"instrument":...,"trading_date":"YYYY-MM-DD"}`) | `magic.market.opening_auction_diagnostic` |
| `MarketBreadth` | `magic.market.market_breadth.request` (`{"source_date":"YYYY-MM-DD"}`) | `magic.market.market_breadth_diagnostic` |

例如技术日 K 诊断的业务 JSON 为：

```json
{"instrument":{"exchange":"Shanghai","code":"600396","asset_class":"Equity"},"interval":"Day","start":null,"end":null,"limit":20}
```

除下述东财妙想默认诊断外，外层 `QueryRequest` 必须同时设置对应的
`preferred_provider` 和 `allow_unadmitted=true`。MA5/MA10/MA20 及资金流分档等源端未提供的可选字段保持
`null`，调用方不得补零。盘后资金诊断中的 `super_large_net`、`large_net` 以及来源缺失
字段同样保持 `null`；排行诊断同时返回 `reported_universe_size` 与 `fetched_count`，不得
把首个来源页解释为完整市场。

东财妙想无需设置 `preferred_provider` 或 `allow_unadmitted`；服务端在启动时检测 Key
并默认选择 `EastmoneyMiaoxiang`。Key 只放在服务进程环境，绝不能放入
`QueryRequest`。例如开盘集合竞价诊断：

```json
{"instrument":{"exchange":"Shanghai","code":"600396","asset_class":"Equity"},"trading_date":"2026-08-14"}
```

返回记录中 `matched_quantity_shares` 和 `matched_amount_cny` 有源值，其余未证明竞价字段
为 `null`，`status="Unavailable"`。市场宽度请求为
`{"source_date":"2026-08-14"}`，只消费五个已证明家数；`listed_total`、`coverage` 和
`maximum_source_skew_millis` 保持 `null`。

2026-08-15 当前工作站真实 gRPC 验证：

| Operation | 结果 | 观测摘要 |
| --- | --- | --- |
| `TechnicalBars` | 返回 1 条，`UNADMITTED` | 600396.SH，2026-08-14 未复权日 K，含 MA5/10/20 |
| `MoneyFlows` | 返回 1 条，`UNADMITTED` | 600396.SH，2026-08-14 五档资金净额；未使用 TDX 成交额冒充 |
| `MarketRankings` | 返回 2 条，`UNADMITTED` | 来源声明总数 5554、首屏抓取 100；两条源时间不同，明确非原子 |
| `PostCloseFlows` | 返回 2 条，`UNADMITTED` | 显式请求 2026-08-14；`super_large_net`/`large_net` 为 `null` |
| `FundFlowSeries` | 东财妙想返回记录，`UNADMITTED` | 600396.SH 日级五档净额；服务端有界截断源端多返回日期 |
| `FuturesDelivery` | 当前 `UNAVAILABLE` | CFFEX Rustls 握手 `unexpected end of file` |
| `Auctions` | 东财妙想返回部分记录，`UNADMITTED` | 2026-08-14：开盘竞价成交量 2,951,900 股、成交额 53,665,542 元；其他字段为空 |
| `MarketBreadth` | 东财妙想返回部分记录，`UNADMITTED` | 2026-08-14：上涨 2400、下跌 2970、平盘 170、涨停 64、跌停 13；总数/覆盖率/偏差为空 |

## 11. gRPC 错误处理

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

服务端把安全的 Protobuf `ErrorDetail` 编码在 trailing metadata
`magic-error-detail-bin` 中：request ID、operation、Provider、reason code 和 retryable。
该自定义 detail 不占用标准 `grpc-status-details-bin`，因此 grpcurl 等标准客户端不会把
它误解为 `google.rpc.Status`。调用方不得依赖自然语言 message 做程序分支。

## 12. 客户端代码生成

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

## 13. 联调检查表

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

## 14. 服务端发布时需要交付给对接方

1. `market.proto` 和 descriptor set 摘要；
2. 服务地址、TLS CA、认证材料；
3. 服务端消息/并发/流/重放限制；
4. 已准入 capability 快照与精确 scope；
5. 每个已启用方法的 canonical request/record schema fixture；
6. TDX 是否仅影子模式及其 admission 状态；
7. 版本升级和字段废弃通知周期。
