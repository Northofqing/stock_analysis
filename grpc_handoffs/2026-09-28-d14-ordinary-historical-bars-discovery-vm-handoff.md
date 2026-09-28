# D14 普通 HistoricalBars 异常日线候选发现：上游合同交接（2026-09-28）

## 当前边界

本地 `src/data_gateway/historical_bars.rs` 的普通日线 `candidate_discovery` 固定返回 `daily_change_discovery_unavailable_v1`。普通 HistoricalBars v1 只把已准入记录转换为 OHLCV 与批次证据；在 BR-171 将异常批次拒绝为 `Internal` / `no_verified_batch` 后，本地拿不到可审阅的原始相邻 bars。既有 300005、90 日样本见 [TDX E2103 交接](2026-09-24-tdx-e2103-local-reconciliation.md)。不能解析错误文本、补造 raw 或借用只适用于 VerifiedOutcomeDue 的 TDX 专用 `OutcomeDailyBars`，否则会把未准入事实伪装成候选。

本地已有候选审阅数据库、7 天 token、revision/observation、同事务确认与同一精确事实的放行门。缺的是普通 HistoricalBars 的资格化发现输入。`daily_change_review::identities` 目前只认 `outcome-provider-sequence-v1`；普通合同须显式扩展版本，不得把 v1 错误响应当作新合同。

## VM 需核实与交付

1. 先核实权威来源能否在 BR-171 拒绝普通日线时，仍通过**独立的只读发现合同**返回原请求身份和完整原始相邻 bars。若源只能返回错误码、无法取得原批次，应明确报告 unavailable，保持本地门关闭。
2. 合同需版本化公布 RPC method、operation/schema、请求和脱敏原始响应；绑定交易所、六位代码、资产类别、请求日期范围、`request_id`、实际覆盖范围、分页完整性、Provider/source、batch ID、`source_at` 与 `observed_at`（源无时间时明确缺失），以及原始记录和稳定摘要。旧 HistoricalBars v1 不得静默增补可选字段后让缺字段落入成功路径。
3. 对上市前后、公司行动和价格变动规则分别给出权威事实、有效区间和覆盖证明。缺上市状态、公司行动或完整相邻交易日时，只能保留候选不可确认；不得由代码前缀、价格跳变或错误文本推断。
4. 对 300005 的 90 日真实请求复现：保存原始 RPC、拒绝原因及独立发现响应；同时验证请求 A 返回 B、混合/重复记录、换价、缺批次、部分页、过期证据和上游错误都不会产出可确认候选。回执列源码提交、测试、公开 bundle/descriptor、Health、运行 PID/二进制哈希及仍不可交付范围。交易时段不得直接替换线上服务；先交付公开合同与 fixture，等本地消费者完成同版编译后再约定部署。

## 本地接收后工作

本仓将做 typed adapter，把原始发现证据绑定到 `QualifiedDailyChangeDiscovery`，在同一数据库事务调用候选 `discover_on_conn`，随后通过 CLI 暴露 CandidateReview/token；普通 finalizer 仍只读取同一精确事实。验证真实 VM 300005 请求经过重启、审阅、确认后仅放行匹配的证券、日期与原始记录；改价、换证券、缺批次、partial、过期和并发确认均保持拒绝。相关定向回归优先使用现有 `task8_` 数据库、Gateway、CLI 用例；CatalogV3 review extension 仍须显式迁移。

此交接只记录上游事实与验收要求；当前普通 HistoricalBars 生产候选发现保持 unavailable，未修改本地生产接线。
