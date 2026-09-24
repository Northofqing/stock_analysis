# Consensus 全线失败（earnings batch rejected）归因

日期：2026-09-23
范围：`monitor-launchd-20260916.stderr.log` 中 1647 条 `[BR-115] <code> earnings batch rejected`（窗口 15:03:51–21:10:40 CST，27 个代码）
结论：**这些调用没有到过我们的 VM（`10.211.55.3:50051`）。** 它们发生在你们本机 `127.0.0.1:18082` 的 provider host 上；`no_current_reports` 是你们自己那份代码的分类，我们的服务端不产生这个码。

## 1. 我们这边固定 provider 实测：Consensus 正常

请求 `magic.market.consensus.request` v1 + `{"instruments":[{"exchange":"Shanghai","code":"688690","asset_class":"Equity"}]}`
时间 2026-09-23 21:13:16 CST

| 项 | 值 |
|---|---|
| admission | ADMITTED |
| selectedProvider | Tonghuashun |
| complete | true |
| 记录 | 1 条 `magic.market.consensus_snapshot`（2026/2027/2028 三个财年，contributor_count 5）|
| trailer | 无（`magic-error-detail-bin` 不存在即成功）|

复现：`target\runtime\probe\call-consensus.ps1 -Codes 688690`（需 `PROBE_TOKEN`）

## 2. 你们的 Consensus 不走 ExternalV1

- `src/data_gateway/grpc_source.rs:3514` `consensus_async` → `query_op` → `ContractProfile::LocalBridgeV1`
- `src/grpc_client/client.rs:1045`：LocalBridgeV1 用 `GRPC_MARKET_ADDR`，默认 `http://127.0.0.1:18082`（`grpc_source.rs:70`）
- `src/grpc_client/external_v1.rs` 只有 SecurityMetadata / GlobalNews / InstrumentNews 三个 operation，**没有 Consensus**
- 因此 Consensus 永远到不了我们这里，与我们 2026-09-23 14:00 的部署无关。

## 3. 契约不兼容（反向实测）

把你们 LocalBridgeV1 的信封原样发给我们（schema `market.consensus` v1，data 为 `{"codes":["688690"]}`）：

`InvalidArgument` / `reason_code=invalid_request` / `consensus requires schema magic.market.consensus.request version 1`

我们只接 `magic.market.consensus.request` v1 + `{"instruments":[...]}`（`docs/integrations/grpc-external-api.md:729`）。
推论：**即使把 18082 改指到我们，这条也不会变成 Internal，而是 InvalidArgument。** 所以日志里的 `internal` 不可能来自我们。

## 4. `no_current_reports` 的产地在你们自己的旧 provider host

- 本仓全量检索 + 全部 479 条历史（`git log --all -S`）无此字符串；你们锁定的上游 revision `75ee2a2` 也无。
- 产出点：`120b90dc:src/data_gateway/consensus.rs`（旧 in-repo provider host，已在 `c16a390b refactor: remove in-repository provider host` 删除）
- 该文件用 Eastmoney 研报实现 Consensus：
  - `const REPORT_WINDOW_DAYS: i64 = 180;`
  - `let begin = today - Duration::days(REPORT_WINDOW_DAYS);`，窗口内（`normalize_reports` 过滤后）为空则返回
    `GatewayError::classified(CAPABILITY, Some(Eastmoney), "unavailable", "no_current_reports", false, "typed provider returned no reports in admitted window {begin}..={today}")`
- 与日志逐字对应：`provider=Some(Eastmoney)`、`retryable=false`。同文件的 `invalid_evidence(...)` 分支对应 136 条 `invalid_evidence`。

## 5. 计数分解

窗口 15:03:51–21:10:40，1647 行，27 个代码：

| 项 | 计数 |
|---|---|
| reason_code=no_current_reports | 1285 |
| reason_code=internal | 211 |
| reason_code=invalid_evidence | 136 |
| reason_code=no_verified_batch | 31 |
| provider=Some(Eastmoney) | 1632 |
| provider=None | 31 |

- 同一行可出现多个码，故各码之和不等于行数。
- `provider=None`(31) 与 `no_verified_batch`(31) 完全重合：这是你们客户端**没解到 trailer** 时的默认值（`grpc_source.rs:718-719` 的 `unwrap_or`），不是服务端分类。
- `internal`(211) 是 `reason_code_static` 对不在封闭词表内的 wire 值的折叠（`grpc_source.rs:1015` / `grpc_client/errors.rs:295`）。我们的封闭词表只有七个：`capability_unadmitted`、`source_precondition_failed`、`invalid_evidence`、`invalid_request`、`internal`、`provider_route_exhausted`、`provider_route_stopped`。

## 6. 归属

1. 不是我们的缺陷：Consensus 在我们这边 ADMITTED 且 complete。
2. 是你们本机 `127.0.0.1:18082` 的旧 `grpc_market_server`（2026-08-30 23:25 构建，`codex/tdx-local-provider-20260923`），其 trailer 走 `grpc-status-details-bin` 而非 `magic-error-detail-bin`，与你们 `2026-09-23-tdx-historical-bars-local-route-investigation.md` 的定位一致。
3. 业务面：那 27 个代码在 Eastmoney 180 天窗口内没有研报，旧服务的规则把它判成「不可用（非重试）」。这是业务态，不是传输故障。

## 7. 下一步（需你们决定，我们不擅自改契约）

- A：把 Consensus 迁到 ExternalV1。需要我们新增一个 `magic.market.consensus.*` 外部 operation，并核对我们的 Tonghuashun 一致预期与你们 `ConsensusData` 的字段差异（你们索引 GD-004 记的「转换固定丢最近报告、日期与目标价」需一并评估）。
- B：Consensus 继续走本机 provider host。则应由其维护方把 `no_current_reports` 的业务语义与 BR-115 的 earnings batch 拒绝策略对齐。

## 8. 本文件不宣称

- 不宣称 18082 服务此刻的状态（我们无法从那台机器发起调用）。
- 不宣称那 27 个代码在其它数据源下也没有研报。
- 不宣称 `internal`(211) 的具体来源，只证明它不是我们封闭词表的产出。

## 9. 复现脚本（我方 scratch，未随交付复制到共享盘）

- `target\runtime\probe\call-consensus.ps1` — 固定 provider 的 Consensus 探针
- `target\runtime\probe\consensus-scan.ps1` — 你们日志的聚合
