# gRPC Gateway 失败审计误记默认提供者（2026-09-26）

性质：下游客户端审计归因缺陷；现有计数不能直接证明 TDX 提供者故障。与 [日线同类修正](2026-09-24-tdx-e2103-local-reconciliation.md) 属于同一类错误，但不改写历史审计行，也不改变 gRPC 准入结果。

## 证据

- `MarketDataGateway::realtime_quotes` 在成功时从批次证据取 provider，在任何失败时却固定回退 `Tdx`；`bridge_for` 连接失败也固定记 `Tdx`。`GatewayError::provider()` 可以是 `None` 或其他真实提供者，因而审计的 provider 与失败证据可能不一致。
- 2026-09-26 约 02:04 CST 对生产库只读查询 `data_acquisition_audit`，在 `id > 3180000` 范围内，`RealtimeMarketQuotes` 有 `Sina / unavailable / no_verified_batch = 3398`、`Tdx / unavailable / no_verified_batch = 847`、`Tdx / unavailable / grpc_bridge_sync_timeout = 4`。这些是审计行数，不是去重请求数；`Tdx` 失败行可能含错误归因，不能据此推断物理 TDX 失败数。夜间行情可能本来不满足五秒时效门，本次没有把 `no_verified_batch` 解释为上游故障。
- 定向回归以不可达本地桥调用公开网关，错误的 provider 为 `None`，但修正前持久审计行是 `Tdx`，对 `Custom` 的断言实际失败（`left: "Tdx", right: "Custom"`）。
- 板块成员同步入口的相同回归也在旧代码下失败：错误 provider 为 `None`，持久审计行却为 `Tdx`（`left: "Tdx", right: "Custom"`）。同一生产库 `id > 3180000` 的 `board-memberships` 有 `Tdx / unavailable / no_verified_batch = 15` 和 `Tdx / unavailable / grpc_bridge_sync_timeout = 2`；这些旧行不能证明物理 TDX 故障。

## 下游处理与边界

`realtime_quotes`、板块目录／成员／资金流、分时形态、分钟线／五档／资金流／证券元数据入口已改用现有路由审计入口：成功沿用批次 provider；失败沿用错误内的 provider；没有 provider 证据时记 `Custom`。请求散列、失败原因、重试属性与失败关闭行为不变。修正只影响新写入行，不追认旧行的真实提供者。部署后的验收应重新按新行观察 `Custom` 与有证据 provider 的比例，并与服务端 request_id 对账；单靠旧 BR-159 行没有原始 request_id/trailer，无法补回物理提供者。

其他 Gateway（公告、个股新闻、全球行情、研究、龙虎榜等）仍有固定 provider 失败回退，须逐路径审计；不能因为本批修正就宣称所有 gRPC 能力的提供者归因已闭合。生产库由上下游共享，统计中可能同时包含服务端和客户端审计，现有行不足以区分两者。

通过 Parallels 控制会话执行只读 `hostname`，确认可直达 `DESKTOP-IMSDKM0`；上游 `C:\DevelopFile\magic-market-data-rs` 当前有其他未提交工作。此问题发生在本项目客户端审计入口，修改 VM 服务端不能纠正旧客户端归因。本次未改 VM 项目或服务；若发现有服务端请求／响应证据不合合同，应在独立范围内直接修复并记录 request_id 与验收结果。
