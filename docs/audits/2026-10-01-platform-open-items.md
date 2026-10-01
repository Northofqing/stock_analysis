# 平台剩余工作与上线门禁（2026-10-01 21:55 CST）

本文按**生产事实、开发验证、外部依赖**分层记录当前欠项。完整目标和阶段依赖见[平台路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)；单次上线的制品与运行证据见[Wave 0 切换记录](../ops/2026-10-01-wave0-monitor-cutover.md)。本文不是上线批准，也不将源码测试等同于真实接收、账本或交易日观察。

## 已有的生产事实

- Wave 0 在冻结源码 `1f0fc6a7f2c90916066a937e128500c532ae1c85` 和获批 activation 下单实例运行。20:35 CST 持久投递库为 `Delivered=981`、`RejectedDurable=3988`、`ManualResolvedRejected=6`、`UncertainManualReview=78`；一条 `AcceptedAuditPending` 仅恢复审计和最终状态，没有重发。
- 21:07 CST 只读健康检查显示进程、心跳、快照均新鲜，但账户 `Frozen`、数据 `Unsafe`，Quote、Kline、MoneyFlow、News、OrderBook 五项缺失，整体 `banner_unhealthy`。因此 Wave 0 是安全切换，不是平台 Ready。
- 21:33 CST 当前生产 client bundle 的只读 ExternalV1 公告探针得到 `ADMITTED/complete`、300 条，provider 批次标识报告总量 722。该探针仅证明有界 RPC 可用；300 条不证明全天全市场覆盖，也不证明生产 monitor 已消费。

## 开发完成但尚未上线

| 工作 | 当前证据 | 下一道门 |
| --- | --- | --- |
| R-08/NewsMonitor/A-11 公告路由 | 开发提交 `c91be0e1` 走 ExternalV1 并核对 typed mTLS provenance；相关库测试 9/9、monitor R-08 binding 1/1 通过。 | 冻结生产基线专属测试、饱和批次拒绝、release 制品、新 activation、人审和生产观察。全量分页仍需上游修复。 |
| NewsMonitor 启动隔离 | `14be9462` 将阻塞构造放入 worker，开发目标测试 1/1；Wave 1 候选正扩至其他同步启动读取和 NewsFlash 审计恢复。 | 候选专属测试与 release 构建；生产启动时计时器和健康快照观察。每轮同步审计恢复仍需单独治理。 |
| M1 产业链 Custom 输入影子 | `55a006ba` 把首个已建 HTTP 请求体与同次保留的 `report_input` 直接比较，隔离 worktree 定向测试 3/3。 | 继续同事实抑制/决策和其他渠道的 exact bytes；本片没有物理 owner、权威回执或 Foundation 晋级。 |

开发分支与生产二进制不是同一版本。Wave 1 候选从已上线的冻结源码单独取最小改动，目前仍在测试、构建和激活材料准备中；Wave 0 的人审只批准了 Wave 0 的精确 activation，不能复用。

## 尚未完成的阶段

| 阶段 | 核心欠项与退出证据 |
| --- | --- |
| M0 数据与事实重基线 | 生产数据五项缺失；LocalBridge 公告方法仍不支持，R-08 强制 CFFEX 来源和完整公告覆盖未验；Global FX、BlockTrades 等无 verified batch。VM 资金流/板块资金流同版验收仍为可重试 `provider_unavailable`，D14 来源身份及区间终态、D17/D20 权威逐日与空结果覆盖仍缺。52 Unit 的源码、部署、真实回执三栏还未逐项闭合。 |
| M1–M2 推送 Foundation 与全目录 | 52 Unit 需逐项分类并核验；其中 required active Unit 要完成同事实 shadow、独立 occurrence、单一物理 owner、持久 intent/finalizer/reconciler、目标渠道真实 `TransportAccepted` 与自然观察，禁用/Starved/Opt-in 项保持既定资格。78 条 `UncertainManualReview` 要按外部身份和人工证据裁定，不能自动重发。CLI、产业链、归因、snapshot、候选板、涨停板、复盘等首批 Unit 均未取得 `Production Verified`。 |
| M3 运行与复盘 | 统一健康/原因/每源恢复、Quiet/Halted、OutcomeTracker、AI 评价与持续运行窗仍需闭合；一次健康快照和启动成功不满足五个真实运行日的观察。 |
| M4 v18 闭环 | `GlobalSchema` authority 尚未接线；Decision/Fill/PaperLedger 的单 owner、point-in-time 历史、费用 v2 回测与账本同口径、旧持仓 seed/cutover 和逐日对账仍未完成。 |
| M5 Gate P | 远端 WORM/Object Lock、至少五年保留与恢复读取、签名日根、重放和成本后样本外证据、故障演练及模拟观察均需真实验收。 |
| M6–M7 研究与前瞻模拟 | v20 候选先逐项经 PRD、ADR、测量与样本外门禁决定是否建设；被批准的能力再实施。候选策略需前瞻 paper 观察，并给出保留、限制或淘汰裁定。 |

M8 扩容和 Web 等仅在有产品需求或实测瓶颈后决定；不属于无条件“全部实现”的清单。上线顺序仍依赖 M0 数据与运行资格 → M1 单 Unit 物理 owner → M2 全目录；M3 可与 M2 交错，M4–M7 按路线图的证据门推进。
