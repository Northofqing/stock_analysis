# gRPC 项目处理结论（GD-001 ～ GD-015）

日期：2026-09-17  
项目：`magic-market-data-rs`  
修复提交：`098021444d7b3c0dea4732c1b5a03e8773047cfb`  
client-bundle：`2026-09-17.1`  
生产端点：`https://10.211.55.3:50051`，TLS authority 为 `magic-market.local`

## 结论摘要

本轮确认两个服务端合同问题并已修复：Hithink 财务报表原始
`fiscal_period` 在 Core/gRPC 投影中丢失。`FinancialStatements` 现新增可选 schema v2，
逐条保留 Provider 原始期间标签；冻结的 v1 继续接受且不改变记录形状。

此外，`provider_attempts` 过去接受任意小写 reason，gRPC 边界还会用 `take(16)` 静默
截断较长链路。现已发布闭合词表、ordinal/布尔组合规则和安全示例；17 项或更多整体
返回无部分 attempts 的 `INTERNAL/internal`。Bundle 同时携带与真实 Health 对应的完整
deployment build identity 及精确 SHA-256 口径。

其余问题主要分为三类：客户端调用了错误 operation/请求形状；客户端聚合或投影丢失了
服务端已经返回的字段与 lineage；需求字段没有可验证的上游证据，服务端不能补造。下面逐项
给出归属和可执行处理方式。

## 逐项结论

| 编号 | 归属/状态 | 结论与动作 |
| --- | --- | --- |
| GD-001 本机 gRPC 运行能力未认证 | 运行配置，已恢复 | 交接探针使用了 `127.0.0.1:18082`，但本项目生产端点是 `10.211.55.3:50051`，并要求 mTLS、Bearer token 和 authority `magic-market.local`。服务曾被发现退出，本轮已重新构建、部署和启动；健康接口为 `live=true, ready=true`，运行 revision 与本提交一致。客户端应读取 bundle 的私有连接材料，不得硬编码旧端口。 |
| GD-002 竞价量比在 TopStock 投影后缺失 | 客户端投影问题 | `CurrentAuctionObservations` 已返回可选 `auction_volume_ratio`。在线响应已验证字段存在。客户端从竞价记录生成 TopStock 时应原样保留；不能从 LimitPools 固定填 `None`，也不能跨批用普通行情补值。 |
| GD-003 盘中主力净流与量比在部分消费者前丢失 | 客户端组合问题 | `MoneyFlows` 已由 Eastmoney 准入。量比在竞价期来自 `CurrentAuctionObservations.auction_volume_ratio`，通用排名可使用独立的 `ProviderTopNRankings(kind=VolumeRatio)`。它们是不同 batch，客户端必须保留各自 evidence 后组合，不能伪造成单一来源字段。 |
| GD-004 Consensus 最近报告、日期和目标价被清空 | 客户端调用/模型边界 | `Consensus` 只提供年度汇总；最近报告与目标价分别由 `ResearchReports` 和 `TargetPrices` 提供。客户端应发起三次独立请求并保留三份 batch lineage，不能要求 Consensus 响应承载不存在的逐报告数据。 |
| GD-005 Earnings EPS 比较缺少同期间和 issuer 绑定 | 服务端字段丢失，已修复 | Hithink 原始 `fiscal_period` 过去被验证后丢弃。现 `FinancialStatements` v2 返回该原始字段，v1 保持兼容。只有同 issuer、同 fiscal year、`FY` 实际值且单位一致时，才可与全年一致预期比较；其他情况 typed skip/unavailable。在线 v2 探针返回 `ADMITTED`、记录版本 2、`fiscal_period=Q2`。 |
| GD-006 R03 缺实时账户输入 | 非行情 gRPC 职责 | 账户权益、持仓、可用资金与成交账本属于券商/账户系统。本项目不能从行情推断或伪造。客户端需接入账户快照并把其 evidence 独立绑定到策略输入。 |
| GD-007 ProviderTopN 本地合同不是实际远端字段 | 客户端请求错误 | 正式 v1 请求必须包含 `kind`、`trading_date`、`limit`、`filter_identity`。`VolumeRatio` 与 `MainNetInflow` 是两次独立请求；只发日期或把一份响应本地拆成两份均不合约。精确 A 股 filter 已写入新版外部文档。 |
| GD-008 T0 时间政策与请求完整性 | 合同已存在；当前 TDX 查询路径有运行故障 | v2 已强制并保留调用方 `requested_at`，代码与回归测试通过。重启后 TDX 事件流为 `agent_connected_production`，但 2026-09-17 的两次 T0 在线查询均因 TDX SmartClient `E2005 retry exhausted` 返回 Unavailable；这不是删除时间限制的理由，也不能声称在线验收已通过。需将其作为 TDX 查询链路运行故障继续处理，客户端当前应按 typed unavailable 关闭依赖 T0 的策略。 |
| GD-009 EconomicCalendar 意图与 `{}` wire 分离 | 客户端请求错误/能力未准入 | `EconomicCalendar` 当前仍未准入，不能发送 `{}`。已发布数据用 `EconomicReleaseObservations(limit,country)`；未来日程用 `EconomicReleaseSchedule(start,end,limit)`。两者证据语义不同，不得互相冒充。 |
| GD-010 搜索聚合丢 provider 失败和调用身份 | 下游审计问题 | gRPC 已返回 typed failure、selected provider 和批次证据。搜索聚合层必须保存每次 provider attempt、请求 identity、reason code 和 retryable；不能只保留最终 router 结果。 |
| GD-011 CloseCall 两次读取破坏输入绑定 | 下游快照问题 | 服务端不能证明客户端两次独立读取属于同一策略快照。客户端应一次捕获所需 batch，保存其 batch_id/observed_at/source_at，再从该不可变输入派生结果。 |
| GD-012 SectorTop 丢板块 batch lineage | 下游投影问题 | 板块聚合必须保留组成 batch 的 provider、batch_id 与逐条 evidence；仅保留板块名称/排名不足以追溯。无需修改现有 gRPC 合同。 |
| GD-013 SectorAnomaly 丢两榜与新闻归因身份 | 下游聚合问题 | 两个榜单与新闻是三个独立查询。客户端必须保存每份输入的请求 identity、batch_id 和 provider，并在派生记录中显式引用，不能压成无来源的布尔结论。 |
| GD-014 公告日期与条数未进入 wire | 客户端调用错误 | 单证券 `Announcements` 需要 instrument/limit，可选 start+end；全市场盘后发现应调用 `MarketAnnouncements(start,end,limit)`。需要“业务日 + 全市场 + 最多 300 条”时不能向 `Announcements` 发送 `{}`。 |
| GD-015 大宗交易本地审计/消费投影未闭合 | 客户端问题 + 来源能力边界 | 当前 `BlockTrades` 是单证券、日期范围和 limit 合同，记录保留真实交易日、可选成交时刻、价格、原始 `DEAL_VOLUME` 数值、金额、买卖席位与 evidence。来源未证明稳定成交行 ID、成交类型、实时确认状态、交收期及独立单位枚举，服务端不能填造 `Agreed/realtime/NextSession`，客户端也不能把 code 当名称或把 f64 强转 u32。需要这些字段时应先取得真实来源证据，再设计 v2。 |

## 已发布合同与请求要点

- `FinancialStatements`：请求 schema 不变；显式 `schema_version=2` 得到记录 v2 和可选
  `fiscal_period`，v1 记录仍不含该字段。
- `MarketAnnouncements`：`{"start":"YYYY-MM-DD","end":"YYYY-MM-DD","limit":300}`。
- `BlockTrades`：单证券 `instrument/start/end/limit`，多证券由客户端有序拆分，并分别保存 batch。
- `ProviderTopNRankings`：必须发送 `kind/trading_date/limit/filter_identity`。
- `Consensus`、`ResearchReports`、`TargetPrices`：三类独立 operation，不共享或伪造 batch。
- `CurrentAuctionObservations.auction_volume_ratio`：可选 Decimal multiple；缺失时保持缺失。

## 验证证据

- `cargo check --workspace --all-targets --all-features --locked --offline`：通过。
- `cargo test --workspace --all-targets --all-features --locked --offline -- --test-threads=1`：通过，0 failure。
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`：通过。
- `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps --locked --offline`：通过。
- workspace doc-test、format、文档链接、BR/HTTP/TDX/gRPC 合规检查：通过。
- 在线健康：`sourceRevision=098021444d7b3c0dea4732c1b5a03e8773047cfb`，
  `contractSha256=0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41`，
  `binarySha256=22a9726aab44473694141ef78b884c56a99549b23d3c3f9381fead66f6510727`。
- 在线 FinancialStatements v2：`ADMITTED`，record schema version 2，真实 `fiscal_period=Q2`。
- 在线 CurrentAuctionObservations：`ADMITTED`，保留 `auction_volume_ratio`；无上游源时间时
  `source_at=null`，不补造。
- bundle 已在目标目录原地执行 `sha256sum -c manifest.sha256`，全部条目 PASS。

## client-bundle 实施计划补件回应

### Provider attempts：已闭合

`grpc-external-api.md` 已给出完整 closed contract：

- ordinal 从 1 开始、连续、无重复，数组按实际尝试顺序排列，长度为 1..=16；
- provider 必须逐字匹配同端点 `GetCapabilities` 的 identity；
- outcome 只允许 `selected`、`rejected`、`failed`；
- 每个 outcome 的 reason、retryable、terminal 合法组合均已逐项列出；
- 未知或冲突状态在服务层构造时即失败，不能参与客户端重试决策；
- 内部 17 项或更多时整体返回 `INTERNAL/internal` 且 attempts 为空，不再截断。

安全投影示例：

```json
{
  "request_id": "REDACTED_REQUEST",
  "operation": "RealtimeQuotes",
  "reason_code": "provider_route_exhausted",
  "retryable": false,
  "provider_attempts": [
    {"ordinal":1,"provider":"Tencent","outcome":"failed","reason_code":"transport","retryable":true,"terminal":false},
    {"ordinal":2,"provider":"Tdx","outcome":"rejected","reason_code":"evidence","retryable":false,"terminal":false}
  ]
}
```

### Deployment identity：已发布并与线上一致

`bundle-metadata.json.deployment_build_identity` 现包含 service version、完整 source revision、
contract SHA-256、binary SHA-256。哈希口径如下：

- contract：`magic_market_grpc_contracts::v1::FILE_DESCRIPTOR_SET` 返回的原始编译后
  `FileDescriptorSet` bytes；
- binary：部署目录中实际运行的 `magic-market-grpc-server.exe` 精确 bytes。

当前 bundle 值与线上 `GetHealth.build_identity` 四字段逐字一致。身份字段只允许整组生成；
缺任一字段、非完整 40 位 commit、非法 service version 或非小写 64 位 hash 都会使 bundle
构建失败。

### RPC 61/62/63 真实只读样本

2026-09-17 部署后通过同一 mTLS/Bearer 生产端点采集：

| RPC | 当前结果 | 安全摘要 |
| --- | --- | --- |
| 61 `CurrentAuctionObservations` | `ADMITTED`, 非空 | Provider route `HithinkFinance`；记录 v1；`auction_volume_ratio=1.8919 Decimal`；上游未证明逐条源时刻，因此 `source_at=null`，未补造。 |
| 62 `EconomicReleaseObservations` | `ADMITTED`, verified-empty | `complete=true`；请求真实携带 `limit=20,country=中国`；batch identity 为 `jin10:REDACTED:economic-release-observations`。结论只覆盖本次滚动窗口。 |
| 63 `EconomicReleaseSchedule` | `ADMITTED`, 非空 | 记录 schema `magic.market.economic_release_schedule_entry@1`；逐条 provider `Fred`；示例 `release_id=2, release_date=2026-09-01`；`source_at=null`，未用日期或 observed_at 合成。 |

Capabilities 同时返回 63 个 operation、62 个 admitted、1 个 blocked；61/62/63 分别绑定
`HithinkFinance`、`Jin10`、`Fred`。

### Local private 61/62：不属于本服务公开合同

计划中的 Local `ChainBatch=61`、`BenchmarkBars=62`、`QueryResponse.source=11` 和
`BenchmarkErrorDetail` 是下游项目冻结的本地桥接合同。本项目发布的是独立 External
63-RPC proto，不能伪造 Local descriptor、构建身份或成功样本。Local 部署证据应由持有
该冻结 proto 和实际 Local endpoint 的项目生成；这不是本服务缺方法，也不应把 External
61/62 按整数重解释为 Local 方法。

### 当前未闭合运行故障

TDX event agent 已连接生产 listener，但 `T0Evidence` 查询仍返回 typed
`Unavailable/[E2005] retry exhausted: max retry reached`，production replay 也暂未同时包含
admitted fast 与 snapshot observations。服务端日志中的直接前因是三个 TDX SmartClient
候选均先返回 empty quotes，随后健康检查收到
`[E2103] response length mismatch: security bar row 0 is truncated`，最终耗尽重试。该问题与
本次 attempts/bundle 合同修复无关，已按真实运行故障保留；在证明是远端坏包还是协议解析
漂移前不放宽 parser，客户端不得将能力声明等同于当前数据可用。

### 验证说明

除上述完整 Rust 检查外，本次新增回归明确覆盖非法 attempt 状态和 17 项原子拒绝；bundle
完整/空 identity 均可解析，部分 identity 明确失败。仓库 Python 辅助测试未运行，因为当前
主机没有 `python`、`python3` 或 `py`；不将其记录为通过。发布脚本的其余等价步骤均已逐项
执行并通过。

## 取舍说明

v2 的优点是保留原始期间身份且不破坏冻结 v1；代价是需要客户端显式升级并正确处理
`null`。没有上游证明的字段继续保持不可用，会减少表面上的“字段完整度”，但避免把本地时间、
默认枚举或内容 hash 冒充 Provider 事实。
