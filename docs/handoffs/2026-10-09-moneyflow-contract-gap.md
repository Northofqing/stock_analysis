# MoneyFlows 合同与 Mac 消费接线缺口（2026-10-09）

本交接依据当前仓库源码的只读调查。ExternalV1 已有隔离获取和准入实现，但仍未接生产实时 consumer。下面的日资金流上下文是可开发范围，尚未实现或上线；本次没有访问凭据、发起外部请求、修改生产数据或调整 capability/freshness。

## 现有实现与边界

| 路径 | 实际行为 |
| --- | --- |
| [monitor overlay](../../src/bin/monitor/main.rs) `:10158` | `fetch_flow_overlay` 仍调用 `bridge.money_flows_async(codes)`；返回的 map 只保留 code 与主力净额，供盘中信号消费。 |
| [GrpcSource](../../src/data_gateway/grpc_source.rs) `:3463`、[旧 converter](../../src/data_gateway/grpc_source/convert.rs) `:886` | 旧 LocalBridge 路线发送批量 `codes`，期待 `code` 和 RFC3339 瞬时时间 `source_at`。设置 `GRPC_MARKET_CLIENT_BUNDLE` 不会自动把此方法切到 ExternalV1。 |
| [External 请求](../../src/grpc_client/external_v1.rs) `:124` | MoneyFlows 只接一个沪深 equity `instrument`，使用 `magic.market.money_flows.request`；不是旧 `codes` 合同。 |
| [隔离 gateway](../../src/data_gateway/external_flow_read.rs) `:1`、`:43` | 已能获取原始观察并调用 `admit_money_flow`；目前只有隔离测试消费，源码明确保留 revision-bound 上游验收门。 |
| [External 日点](../../src/data_gateway/external_flow.rs) `:26`、`:236`、`:276` | 返回 `ExternalMoneyFlowPoint.source_date` 与五项净额；准入允许上一交易日至今日。不存在分钟截止瞬时或日终完成字段。 |
| [DataMode](../../src/monitor/data_mode.rs) `:323`、[日资金流领域](../../src/capital_flow.rs) `:53` | MoneyFlow 仍按未接真实 provider 的辅助能力处理；现有 `MoneyFlowDay` 需要 `main_pct`，其投影依赖 `main_ratio_percent`。External 五净额没有这个事实。 |

`observed_at` 在 30 秒内只证明刚观测，不能证明资金流刚发生。`source_date` 不能转换成真实瞬时时间；也不能从五净额拼造 `main_pct` 或强行生成现有 `MoneyFlowDay`。静态合同本身不能判定该日点是日终完成值还是当日未完成累计值，因此也不能宣称实时 MoneyFlow 已恢复。

## Windows 下一步：提供真实合同与同版原件

- 确认并交付五净额的单位、累计/单日定义，以及 source_date 对应的数据截止和完成语义。
- 提供与实际服务 revision、descriptor、部署 executable 精确绑定的版本材料；同一合格连接上的 MoneyFlows/Eastmoney capability 必须唯一、Admitted、runtime_available 且无 blocker，并提供真实完整业务原件及 unavailable 等反例。仅有 Health ready 或 capability 名称不构成数据可用证明。
- 若目标是盘中/分钟消费，需要版本化合同给出真实资金流截止瞬时、更新节律和累计窗口，不能改用接收或 observed_at 时间充当源时间；同时确认实际证券覆盖与逐证券缺失状态。

对应现有门：[flow capability/response gate](../../src/grpc_client/external_flow_read.rs) `:198`、`:240`；[record admission](../../src/data_gateway/external_flow.rs) `:278`。

## Mac 下一步：按真实语义接线

- 可先开发独立、只读、有界、逐证券的日资金流上下文 consumer，复用隔离 gateway，保留请求、Health/capability、wire、证券、source_date 与各自 evidence；显示明确的“截至日期”和缺失/过期状态。这项范围当前尚未实现。
- 日上下文不签发实时 MoneyFlow freshness，不替代盘中 overlay，不合成时间戳、比例或跨批 provenance。盘中接线等待上述真实瞬时合同及同版原件，再开发独立准入/保留证据的适配。
- 上游版本身份变更时，本地 compiled identity 必须随经过验证的版本材料匹配并正常重建；不能从首个响应学习 expected identity，不能放宽现有身份门以接纳新服务。

当前 expected identity 编译自 [bundle-metadata](../../contracts/external_v1_current/bundle-metadata.json) `:17`，当前 revision 为 `841e4ae7a9df62be0c4536fa9009c66089d282bf`；[build identity qualification](../../src/grpc_client/build_identity.rs) `:16`、`:300` 逐项匹配 service version、source revision、descriptor 和 executable hash。该仓库值不是本次对远端实际部署的观测。

验证范围：源码与调用点检查、文档内容检查和 `git diff --check`；没有执行 Cargo、真实 RPC、生产写入或上线动作。
