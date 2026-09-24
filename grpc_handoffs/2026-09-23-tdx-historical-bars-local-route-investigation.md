# Tdx HistoricalBars：下游对账与实际生产路由（2026-09-23）

承接 [下游索取原始 gRPC 证据](2026-09-23-tdx-historical-bars-e2103-downstream-reply.md) 和 [上游修复回复](2026-09-23-tdx-historical-bars-e2103-upstream-reply.md)。前半记录是修复前的只读调查；末尾另记本机 provider-host 的修复与验收。历史取证与新请求的 ID 严格分开。

## 结论

`audit_id=2926658` 属于 **monitor → 本机 `127.0.0.1:18082` 的旧 `grpc_market_server`** 链路，不是 monitor 直连 `10.211.55.3:50051`。本机服务进程 PID 654 使用 2026-08-30 23:25 构建的 `target/release/grpc_market_server`，并直连 TDX `:7709`。它没有随着 2026-09-23 14:00:33 CST 的上游 VM 部署更新；14:45 的本机 RPC 仍复现 E2103。因此，上游实例日志查不到这个业务窗口，不能用来判断本机服务的 handler 是否执行。

本机服务的日志在 `2026-09-23T02:16:04Z` 连续出现 `[E2103] security bar row 0 is truncated` 和 `no alternative server available`；monitor 在同一秒记录 `audit_id=2926658`，随后跳过 688277 的模拟卖出评估。这是强时间关联，但历史日志没有 request_id，不能把其中某一条 TDX 库日志与该审计行做一对一证明。

## 历史调用能回传的证据

| 下游请求项 | 对 `audit_id=2926658` 的结果 |
| --- | --- |
| `QueryRequest.context.request_id` | **无法恢复**。`GrpcMarketClient::query` 为调用生成独立 ID；现有 monitor 日志、本机服务日志及 `data_acquisition_audit` 行均未存它。审计行只有不可逆的 `request_hash`。 |
| 未包装的 `status.code()` / `status.message()` | **无法恢复原文**。monitor 日志只剩 `GrpcError::Internal` 的固定展示词“服务端内部错误”；客户端转换时丢弃原始 Status。 |
| 原始 trailing metadata | **无法恢复**。旧日志/审计表均未存 trailer；客户端只尝试解码后映射，原始字节不落盘。 |
| 目标 | monitor 日志记录 `server=http://127.0.0.1:18082`；`lsof` 证实 monitor PID 74628 与本机 `grpc_market_server` PID 654 在此端口建立 TCP 连接。不是 `10.211.55.3:50051`。 |
| 调用时间 | 审计行 `observed_at=2026-09-23T02:16:04.340Z`，即 10:16:04.340 CST；这是审计记录时间，不冒充精确 RPC 发出时刻。 |

`data/stock_analysis.db` 的 `data_acquisition_audit` 第 2926658 行记录 `HistoricalDailyBars / Tdx / unavailable / no_verified_batch / retryable=1`，但表结构没有 request_id、status 或 trailer 字段。`src/data_gateway/historical_bars.rs` 在错误时把审计 provider 默认写为 Tdx；因此审计中的 `provider=Tdx` 也不能单独证明远端实际选择了 Tdx。现有客户端 `src/grpc_client/errors.rs` 把原始 request_id 转成 SHA-256 关联值，并把不在封闭词表内的状态文案遮蔽，历史原文不能从这些值反推。

## 新的本机只读复现（与历史调用分开）

2026-09-23 14:45:48 CST，用仓库的 `contracts/local_bridge_v1/market.proto` 对 `127.0.0.1:18082` 发出一次 `HistoricalBars`，请求 `codes=["688277"], days=90`，新建 request_id `codex-local-historical-bars-20260923-1445`。`grpcurl -v` 返回：

```text
Code: Internal
grpc-status=Internal
grpc-message=取数失败: 日线 Gateway 不可用 (688277): ...
  attempts=[Tdx=failed:Protocol:TryNext:[E2103] response length mismatch: security bar row 0 is truncated,
            Tencent=failed:Quality:TryNext:..., Sina=failed:Quality:TryNext:...,
            Baidu=failed:Transport:TryNext:... status code 403]
grpc-status-details-bin: Ciljb2RleC1sb2NhbC1oaXN0b3JpY2FsLWJhcnMtMjAyNjA5MjMtMTQ0NRABIhFub192ZXJpZmllZF9iYXRjaCgB
magic-error-detail-bin: absent
```

`grpcurl` 同时报告 `grpc-status-details-bin mismatch`。将该 trailer 按本地 proto 的 `ErrorDetail` 解码得到：

```text
request_id: "codex-local-historical-bars-20260923-1445"
operation: OPERATION_HISTORICAL_BARS
reason_code: "no_verified_batch"
retryable: true
```

这次调用证明当前本机服务会产生 `no_verified_batch` 包装及 E2103；它的 request_id **不是** 10:16 的历史 request_id，也不应拿去检索上游 VM 日志。`grpc-status-details-bin` 与下游文档要求的 `magic-error-detail-bin` 是不同字段。

另以文档要求的无 `start/end`、`allow_unadmitted=true` 形状，从本机对 `10.211.55.3:50051` 发出一次只读诊断请求；12 秒内未建立连接（`context deadline exceeded`）。这只说明本机在该次探测中无法直连 VM，不能据此推断 VM 服务本身故障，也不能把它当作上游修复的本机验收。

## 修复前的处理边界

当前仓库已在 `c16a390b` 移除 `grpc_market_server` 源码，现存二进制是旧产物；仅更新 `10.211.55.3:50051` 不会更新本机 `127.0.0.1:18082`。要恢复这条生产依赖，需要由本机 provider-host 的维护方更新并验证其 TDX 处理和错误 trailer 合同，或为 HistoricalBars 设计并验证单独的合格路由。不能仅把整个 monitor 的 `GRPC_MARKET_ADDR` 改到 VM：它承载其他 LocalBridge operation，须先逐项验证合同兼容与回滚。修复后以此前失败的标的检查 admitted 日线、模拟卖出评估和真实窗口 `unavailable` 计数。

未来若还需按单次 RPC 与上游对账，应在发出请求的客户端边界记录 request_id、目标和时间，并在失败时保留经安全处理的原始 code、reason 与 trailer 存在性；一对一原始 trailer 取证需使用有界的专用诊断探针。该改动只能帮助以后取证，无法补回 `audit_id=2926658` 已丢失的原始字节。

## 本机修复与验收（2026-09-23 16:12 CST）

旧 `grpc_market_server` 源码已不在主线，因此在隔离维护分支 `codex/tdx-local-provider-20260923`、提交 `1337bed7` 中，从原锁定的上游 `magic-tdx-rs` 版本 `75ee2a2` 回移 [上游修复提交 `98207a4`](https://github.com/Northofqing/magic-market-data-rs/commit/98207a4497d2012883b6b1f4b0cfb19679353ace)：识别“声明行数但无行数据”的服务器响应、拉黑并有界换台，且采用上游当天验证的七台优先服务器。其余 Magic 提供者仍用原锁定版本。旧服务的 LocalBridge proto 快照及 vendored TDX 源码指纹随维护分支保存，避免使用会变化的 ignored `client-bundle` 和把本地回移版误报为原 Git 提交。

针对性验证：`magic-tdx-rs` 的伪 800 行响应回归测试通过；来源标识单测通过；`cargo build --offline --release --bin grpc_market_server` 成功；`git diff --check` 与相关文件的 rustfmt 检查通过。新二进制 SHA-256 为 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`，旧二进制以 `target/release/grpc_market_server.pre-tdx-20260923` 备份，SHA-256 为 `9ed61c0917191adf7670d08b89e3c95217d980a6a24f944a20ff3891d36e44e1`。通过 launchd `kickstart -k` 重启；验收时由 PID 96428 监听 `127.0.0.1:18082`。启动约 7 分钟，主要耗在旧服务对约 300 万条审计链的两次全量校验；这段时间端口未监听。

新请求的实际结果（均为 `127.0.0.1:18082`，请求中没有 `start/end`）：

| 新 request_id / 时间（CST） | 请求 | 结果 |
| --- | --- | --- |
| `codex-local-historical-bars-20260923-1544` / 15:47:43 | 688277，90 天 | E2103 已不出现；TDX 返回数据，但 BR-092 因 2026-07-15 后跳至 2026-07-30 的交易日断档拒绝。其他提供者也未形成有效批次，RPC 仍为 `Internal/no_verified_batch`，最终审计 provider=Baidu。 |
| `codex-local-historical-bars-20260923-short` / 16:04:30 | 688277，5 天 | `ADMITTED`，`selectedProvider=Tdx`，`complete=true`，5 条，2026-09-17 至 09-23。 |
| `codex-local-historical-bars-20260923-control` / 16:12:01 | 600519，90 天 | `ADMITTED`，`selectedProvider=Tdx`，`complete=true`，90 条，2026-05-20 至 09-23。 |

以服务恢复后的审计起点 `id >= 2972000` 查询到 2026-09-23 16:12 CST：`HistoricalDailyBars / Tdx / available` 共 216 条（5 条批次 215 次、90 条批次 1 次），`HistoricalDailyBars / Tdx / unavailable` 为 **0**；另有 **1** 条 `Baidu / unavailable / router_sources_exhausted`，即上表手工发出的 688277 长窗口请求。这些计数说明 E2103 路径已恢复，不代表所有标的、所有窗口都已可用。

**剩余问题：**688277 的 90 天批次仍被 BR-092 拒绝。尚无权威停复牌证据解释 7 月的缺口，不能放宽完整性校验或把它归咎于本次两字节 E2103 故障。需分别核对交易所停复牌事实、TDX 原始日期序列和其他可信日线源，再决定是修正数据源还是为真实停牌建立可验证的例外。旧本机服务的失败 trailer 仍使用 `grpc-status-details-bin`，并出现 `grpc-status-details-bin mismatch`，没有文档要求的 `magic-error-detail-bin`；历史 `audit_id=2926658` 的原始 trailer 依旧无法补回。
