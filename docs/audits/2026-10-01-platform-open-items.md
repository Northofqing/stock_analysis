# 平台剩余工作与上线门禁（2026-10-01 22:45 CST）

本文按**生产事实、开发验证、外部依赖**分层记录当前欠项。完整目标和阶段依赖见[平台路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)；单次上线的制品与运行证据见[Wave 0 切换记录](../ops/2026-10-01-wave0-monitor-cutover.md)。本文不是上线批准，也不将源码测试等同于真实接收、账本或交易日观察。

## 2026-10-02 Windows 交接更新

本次已读取 Windows 交接和 2026-10-01.3 公开包。10/1 23:55:58 CST Mac最小验证exit0：manifest9/9、TLS/认证、Health live/ready及四项身份一致，Capabilities129条；未做本轮业务RPC或生产切换。服务source67c832e4、descriptor abf28a3e、binary9302a303与根/生产/Wave1旧编译pin不同，现有Rust来源资格不能因此通过。新包父目录凭据路径也不满足Rust loader边界。先准备自包含独立探针包、同步版本化信任和历史回执策略、封存公开构建输入，后做当前同版业务验收及独立后继候选。Wave1获批元组保持精确，切换前以实际来源门禁判断。

具体任务、责任、前置和验收见[Windows交接开发任务](../superpowers/plans/2026-10-01-windows-grpc-development-plan.md)。历史日线已有Hithink start/end公开合同，欠Mac exact-window映射、完整coverage/PIT与immutable读取；资金流只有新能力登记，缺同版业务成功证据；公告全量/D14/D17/D20权威合同及R08确认交割仍未补齐。CFFEXv2 Planned合同与Mac分离实现已有，保留确认语义缺口。新增官方发布64/65为独立后续切片，不能关闭M0–M7。

下方22:45及更早运行事实保留各自时间点，不代表本轮重新检查后的生产快照。

## 已有的生产事实

- Wave 0 在冻结源码 `1f0fc6a7f2c90916066a937e128500c532ae1c85` 和获批 activation 下单实例运行。20:35 CST 持久投递库为 `Delivered=981`、`RejectedDurable=3988`、`ManualResolvedRejected=6`、`UncertainManualReview=78`；一条 `AcceptedAuditPending` 仅恢复审计和最终状态，没有重发。
- 21:07 CST 只读健康检查显示进程、心跳、快照均新鲜，但账户 `Frozen`、数据 `Unsafe`，Quote、Kline、MoneyFlow、News、OrderBook 五项缺失，整体 `banner_unhealthy`。因此 Wave 0 是安全切换，不是平台 Ready。
- 21:33 CST 当前生产 client bundle 的只读 ExternalV1 公告探针得到 `ADMITTED/complete`、300 条，provider 批次标识报告总量 722。该探针仅证明有界 RPC 可用；300 条不证明全天全市场覆盖，也不证明生产 monitor 已消费。

## 开发完成但尚未上线

| 工作 | 当前证据 | 下一道门 |
| --- | --- | --- |
| R-08/NewsMonitor/A-11 公告路由 | 冻结生产基线 Wave 1 候选 `3a3a48f8` 走 ExternalV1、核对 typed mTLS provenance，并拒绝触及 300 条上限的批次；候选定向库/monitor 测试及 release dry-run 通过。市场公告组 9 个用例通过，另 1 个 RPC fixture 因沙箱禁止监听本机端口而未取得测试结果。 | 用户已批准精确 activation，但尚未安装；2026-10-02 09:00 CST 后按切换门禁部署并观察。全量分页及 R-08 强制 CFFEX 仍需上游修复。 |
| NewsMonitor 启动隔离 | 同一 Wave 1 候选将同步受众/元数据/去重/信号状态和 NewsFlash 启动审计恢复放入 blocking worker；启动计时器目标测试 1/1、release 构建通过。 | 生产新 PID 的启动计时器和健康快照观察；每轮同步 NewsFlash 审计恢复仍需单独治理。 |
| M1 产业链输入/决策影子 | `55a006ba` 把 Custom 首个已建 HTTP 请求体与同次 `report_input` 核对（隔离定向 3/3）；`071324ac` 类型化记录两 timer 的抑制/发送决策和报告输入相等证据（隔离定向 2/2，旧投递回归 13/13）。 | 继续全渠道 exact bytes 和完整同事实语义；尚无物理 owner、权威回执或 Foundation 晋级。 |

开发分支与生产二进制不是同一版本。Wave 1 的隔离候选从已上线冻结源码只取三个可执行输入变更；735 项清单 SHA-256 `a63f6f71212c5138d087f70ae6a1cd7719dfdfa039d8d915f4231d909f28915d`、release monitor SHA-256 `54efbc4c5b2ac8c772f13ddd9f1f25e628bc6a82e012c7386282f3d1263f1d57`、配置哈希 `8c89a6aa1c0baf9f6b3a7891c5cd05e9781150eb8b951f6f7478872ccf0a6d17`、activation SHA-256 `1e7f92b7c26d89559817ea6eab43c6b92e0becf9d057b9a0b934f60a2942efe6` 已复核，用户在本聊天批准该精确候选，生效为 2026-10-02 09:00 CST。22:38 CST 只读生产预检仍为 Wave 0，`Frozen/Unsafe`。切换前必须重查动态门禁和实际生产根配置哈希；未到生效时刻不得启动新进程。详细材料保存在隔离 worktree `.worktrees/wave1-narrow-20261001/.planning/wave1-narrow-20261001/activation-review.md`。

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
