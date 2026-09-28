# M1 首批 Unit 任务卡（2026-09-28）

状态：执行顺序草案。身份取自冻结的 [65-kind / 52-Unit v1 目录](push-capability-catalog.v1.json)，当前源码另有 `NewsAiAnalysis` 的 [v2 增量](push-current-kind-delta.v2.json)。每张卡在变更 physical owner 前须核本轮源码、部署和真实回执；不能把此表视为已激活。

2026-09-28 定向 `cargo test --locked --offline --lib push_foundation::readiness_probe_tests::` 7/7 通过，仅证明冻结 v1 目录的 readiness inventory、分母和缺事实归类逻辑；它不证明生产 readiness 或任何 Unit 的真实 receipt。

## 共同验收门

每个 Unit 先保存同一份 `PreparedFacts`，shadow 比较 occurrence、业务日、主体、事实来源、规则/模板版本、抑制原因及 exact payload bytes，不重拉 provider、不再调用 LLM、不写 cursor、不触碰 sink。新 owner 的 intent、跨 business/durable DB finalizer、reconciler、activation manifest、readiness、Draining 回退必须在该 Unit 故障矩阵中闭合。

故障矩阵至少覆盖：事实缺失/拒绝；重试与重复 occurrence；sink 明确拒绝；sink 超时且结果未知；`TransportAccepted` 之后、finalizer 之前崩溃；跨日重启恢复；旧 owner 与新 owner 同时想发送。完成只认目标渠道的真实 `TransportAccepted`、相应 durable receipt 和业务完成游标一致；`Uncertain` 留人工裁定，不自动重发。每次物理 owner 晋级只改变一个共享 Unit，一个交易日最多一个；高风险 Unit 观察两个 eligible 运行日，然后清理旧完成路径。

## 顺序与每 Unit 的特殊验收

| 波次 | Unit 与 producer | 旧 completion owner / 明确缺口 | 首个可验收切片 |
| --- | --- | --- | --- |
| 0 | Foundation 及 `MU-p01` conformance | P01 已有 durable business-date claim；先证明状态机可接现有意图与恢复。 | `P01/N02` 的已完成事实、崩溃点、异步渠道接收逐一与新 adapter 对齐；本波不切换新的物理发送。 |
| 1 | `MU-cli-single`、`MU-cli-summary`、`MU-cli-chain` | enum 外 CLI，`Option<AnalysisResult>` / `run()` 返回值 / `Result<()>` 只是本地执行结果；无持久通知 cursor。 | 先确认每个 invocation 与目标渠道的独立 completion identity。单票、汇总、产业链三种 payload 不互当完成；CLI dry-run 与正式发送严格隔离。必要时先拆三张独立 activation 卡。 |
| 2 | `MU-chain-preopen`、`MU-chain-post-close` | `CHAIN_PREOPEN_LAST` / `CHAIN_POST_LAST` 是进程内 calendar-date cursor，timer 无交易日 guard。 | 以 verified business date、窗口和同一报告 snapshot 建 occurrence；跨零点和重启不可重发，同分钟文件覆盖不能改变已准备 payload。与 CLI chain 共享事实，不共享错误 completion。 |
| 3 | `MU-attribution-daily`、`MU-g5b-attribution` | `ATTRIBUTION_LAST_RUN` / `G5B_LAST_RUN` 是进程内日期位；bool/L4 结果不证明外部接收。 | 两个 kind 各自有 report revision、完成 receipt 和失败重试；报告生成成功后 sink 失败不能推进日期。若共同依赖同一次 attribution 计算，仅采集一次事实。 |
| 4 | `MU-snapshot-stale` | startup 与 timer 共用 `SnapshotReminderGate` 内存态。 | 两入口用同一 business occurrence；缺 confirmed snapshot、超时及启动竞态只产生一个 intent；重启后不由 `LAST` 复位造成双发。 |
| 5 | `MU-auction-candidates` | `AuctionRepush`、`CandidateBoard`、`CandidateInvalidated` 共用 `post_close_candidates_notified` 和快照尾行 code 集，双层推进非原子。 | 将三种子决定和 board snapshot revision 固定为一个共享 Unit；任一子通知失败后保留未完成子集，不丢失 invalidated delta，不重复已接收子项。 |
| 6 | `MU-limit-boards` | first/second/third-plus 共用 `board_notified[session,code]` 与空 code 的 L4。 | 每个 code、session、阶段产生独立 occurrence；阶段升级不被旧阶段 cursor 吞掉，也不因重启倒退到 first。 |
| 7 | `MU-paper-review-daily` | auto/manual/push 路径与 `ReviewScheduleState`、cooldown/L4 并存；PaperReview kind 还有异质 noon producer。 | 以同一个日复盘事实只设置一个物理 owner，先对齐 auto/manual 的 task identity、补推及 exact bytes，再裁决 push 路径旧完成状态。 |
| 保留 | `MU-paper-review-noon` | `NOON_SNAP_LAST` 对应的 noon 结构受时间窗口阻断；冻结目录标 `STARVED`。 | 保持 Starved，记录明确的缺失事实/窗口；不通过补造快照或复用 daily receipt 把 noon 晋级。 |

## 每张实现卡的字段

动手前逐项填写：当前源码 commit；运行制品 SHA；`producer_id` 与 `PushKind`（enum 外写 `none`）；business date/window；source batch 与 freshness；旧/new completion owner；intent key；目标 sink 与接受语义；shadow diff hash；注入的失败点；Draining 回退；真实 receipt id；两个运行日观察结果；旧路径 cleanup commit。以上字段缺任一项，状态保留 `Code Ready` 或 `Shadow Ready`，不能写 `Production Verified`。

来源：[平台路线图 M1–M2](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)、[当晚事实基线](../audits/2026-09-28-platform-m0-baseline.md)。
