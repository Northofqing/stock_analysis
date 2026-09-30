# M0 B04：2026-10-01 凌晨生产运行窗（只读）

采样时间：2026-10-01 02:47–02:49 CST。运行根为 `/Users/zhangzhen/.local/share/stock-analysis-runtime`；`launchctl` 的 `com.stockanalysis.monitor` 仍为 PID `17089`，磁盘 release monitor SHA-256 为 `a2589f0115d6f3ee89bf11d7714c7bd4bce58f6d3e5bcbc12b06831ffc5b8b34`。本次只读取健康、日志和 durable SQLite；没有切换 binary、activation、投递 owner 或写入业务数据。

## 当前健康与来源

02:47:13 CST 从该运行根执行旧版 `monitor --health --json`，退出码 1：`monitor_running=true`、`snapshot_fresh=true`，但 `status=unhealthy`、`reason_code=banner_unhealthy`、`account_mode=Frozen`、`data_mode=Unsafe`、`account_metrics_complete=false`，缺 `Quote/MoneyFlow/News/OrderBook`。该旧版健康覆盖仅为 `banner_account_data_only`；它不能给各来源的恢复时间、VM ExternalV1 能力或真实渠道接收作证明。

读取 `logs/monitor-launchd.stderr.log` **末尾 4 MiB**（跳过可能截断的首行），样本约 18,222 行，日志墙钟约 09-30 19:54 至 10-01 02:48 CST。下表是按 `[DataGateway][route] outcome/reason_code` 计数的日志样本，既不是全交易日分母，也不能把 `available` 直接解释为业务投递成功。

| route | 该样本的主要计数 | 最近可用与不可用边界 |
| --- | --- | --- |
| 四家 raw GlobalNews | Eastmoney `available/accepted` 33、CLS/Jin10/ThePaper 各 35；四家各有 `unavailable/external_transport_unavailable` 35 | 四家最后 `available` 均为 01:03:11，首次 `external_transport_unavailable` 均为 01:12:15；02:45:40 最近一次仍为该错误。 |
| SecurityIdentity | `available/accepted` 35；`unavailable/external_transport_unavailable` 35 | 最后可用 01:03:10，首次不可用 01:12:13，02:45:37 最近一次仍不可用。 |
| R-08-announcements | `invalid_request/invalid_request` 53 | 01:10:12 最近一条；这是旧部署 adapter，候选源码修复须经新生产运行验证。 |
| board-memberships | `available/accepted` 1,803；另有 `no_verified_batch` 与 `grpc_bridge_sync_timeout` 各 1 | 02:47 日志仍有 Tdx 可用批次；不抵消上述缺失能力。 |
| consensus | `available/accepted` 534、`unavailable/no_current_reports` 713、`unavailable/invalid_evidence` 216 | 混合状态，不能声称全源可用。 |

该日志窗另有 `OpeningStaticResident failure=external_transport_unavailable` 202 次、`PaperLedger is not activated` 822 次。它们是当前旧进程运行观察；不能据此判断 VM 新合同交付状态或擅自 seed PaperLedger。

## 投递与欠账瞬时值

对正在被 PID `17089` 打开的 `data/durable_delivery.sqlite3` 使用 `sqlite3 -readonly` 聚合 `delivery_decisions`：`Delivered=980`、`RejectedDurable=3988`、`ManualResolvedRejected=6`、`UncertainManualReview=78`，无其他状态行。78 条不确定项分布为 `DataMode=73`、`CloseCall=3`、`T0Advice=1`、`WatchlistTracking=1`；未检查外部渠道回执，不能裁定或重发。相比 [09-29 候选记录](../ops/2026-09-29-monitor-rollout-candidate.md)的 01:04 瞬时值，`Delivered` 增加 139、`RejectedDurable` 增加 1，`UncertainManualReview` 保持 78；聚合变化不等于本轮迁移 Unit 的真实送达。

## B04 裁定

本次关闭了“旧生产此刻运行什么、哪个可观测来源转为不可用、durable 不确定水位”的只读取证部分。M0 B04 **仍未完成**：尚缺新候选部署后的同版来源恢复快照、交易窗口能力与实际渠道接收对账。01:12 左右的同步不可用与外部 transport 有关，但仅凭本地日志不能断言 VM 的具体根因；需上游同 build Health/Capabilities/RPC 样本及 Mac 切换后新进程观察。
