# M1 首批 Unit 任务卡（2026-09-28）

状态：执行顺序草案。身份取自冻结的 [65-kind / 52-Unit v1 目录](push-capability-catalog.v1.json)，当前源码另有 `NewsAiAnalysis` 的 [v2 增量](push-current-kind-delta.v2.json)。每张卡在变更 physical owner 前须核本轮源码、部署和真实回执；不能把此表视为已激活。

2026-09-28 定向 `cargo test --locked --offline --lib push_foundation::readiness_probe_tests::` 7/7 通过，仅证明冻结 v1 目录的 readiness inventory、分母和缺事实归类逻辑；它不证明生产 readiness 或任何 Unit 的真实 receipt。

2026-09-28 `eff7ce6d` 修复定时分析入口：将 `build_stock_list` 已取得的宏观背景和涨停代码集合传入 `AnalysisPipeline`，与手动单次入口保持相同的分析输入。该修复只覆盖 `MU-cli-single` / `MU-cli-summary` 的定时来源上下文；独立 completion identity、durable receipt 和物理 owner 迁移仍未完成。

2026-09-29 重核当前源码：`pipeline/completion.rs` 已用 `StockAnalysisOutcome`、`SummaryCompletion` 将保存与 BestEffort 通知分开，`AnalysisNotification::Unknown` 保留发送开始后的未知结果；`app/modes.rs` 与 `app/schedule.rs` 均检查 `ensure_cli_success()`。`summary_notify.rs` 和单票发送会区别全部弱接受、部分弱接受、全部失败及无渠道。这关闭了“发送失败仍按 CLI 成功返回”的局部代码缺口，但弱接受没有持久 `TransportAccepted` authority，重启后也没有独立通知 cursor。

## 共同验收门

每个 Unit 先保存同一份 `PreparedFacts`，shadow 比较 occurrence、业务日、主体、事实来源、规则/模板版本、抑制原因及 exact payload bytes，不重拉 provider、不再调用 LLM、不写 cursor、不触碰 sink。新 owner 的 intent、跨 business/durable DB finalizer、reconciler、activation manifest、readiness、Draining 回退必须在该 Unit 故障矩阵中闭合。

故障矩阵至少覆盖：事实缺失/拒绝；重试与重复 occurrence；sink 明确拒绝；sink 超时且结果未知；`TransportAccepted` 之后、finalizer 之前崩溃；跨日重启恢复；旧 owner 与新 owner 同时想发送。完成只认目标渠道的真实 `TransportAccepted`、相应 durable receipt 和业务完成游标一致；`Uncertain` 留人工裁定，不自动重发。每次物理 owner 晋级只改变一个共享 Unit，一个交易日最多一个；高风险 Unit 观察两个 eligible 运行日，然后清理旧完成路径。

## 顺序与每 Unit 的特殊验收

| 波次 | Unit 与 producer | 旧 completion owner / 明确缺口 | 首个可验收切片 |
| --- | --- | --- | --- |
| 0 | Foundation 及 `MU-p01` conformance | P01 已有 durable business-date claim；先证明状态机可接现有意图与恢复。 | `P01/N02` 的已完成事实、崩溃点、异步渠道接收逐一与新 adapter 对齐；本波不切换新的物理发送。 |
| 1 | `MU-cli-single`、`MU-cli-summary`、`MU-cli-chain` | enum 外 CLI 已有本轮保存/通知分离及失败返回，但仅是进程内 BestEffort 观察；无持久通知 cursor 或权威接收回执。`MU-cli-chain` 仍须独立核对。 | 为每个 invocation 与目标渠道建立独立 completion identity。单票、汇总、产业链三种 payload 不互当完成；CLI dry-run 与正式发送严格隔离。必要时拆三张 activation 卡。 |
| 2 | `MU-chain-preopen`、`MU-chain-post-close` | 当前由持久 `ChainScheduleStore` 管理 calendar-date 发送尝试、弱接受、未知待裁定及错过窗口；两个 timer 已有交易日 guard。弱接受仍不等于目标渠道权威回执，报告事实/载荷未固定为同一份 `PreparedFacts`，同一业务日跨 calendar date 及同分钟文件覆盖仍需核对。现有盘前 shadow 仅比较调度策略。 | 以 verified business date、窗口和同一报告 snapshot 建 occurrence；跨零点和重启不可重发，同分钟文件覆盖不能改变已准备 payload。与 CLI chain 共享事实，不共享错误 completion。先补盘后调度策略对比，再用一次 preparation 的同一份事实、抑制原因和 exact bytes 完成两个 Unit 的 shadow；保留旧发送 owner。 |
| 3 | `MU-attribution-daily`、`MU-g5b-attribution` | `ATTRIBUTION_LAST_RUN` / `G5B_LAST_RUN` 是进程内日期位；bool/L4 结果不证明外部接收。 | 两个 kind 各自有 report revision、完成 receipt 和失败重试；报告生成成功后 sink 失败不能推进日期。若共同依赖同一次 attribution 计算，仅采集一次事实。 |
| 4 | `MU-snapshot-stale` | startup 与 timer 共用 `SnapshotReminderGate` 内存态。 | 两入口用同一 business occurrence；缺 confirmed snapshot、超时及启动竞态只产生一个 intent；重启后不由 `LAST` 复位造成双发。 |
| 5 | `MU-auction-candidates` | `AuctionRepush`、`CandidateBoard`、`CandidateInvalidated` 共用 `post_close_candidates_notified` 和快照尾行 code 集，双层推进非原子。 | 将三种子决定和 board snapshot revision 固定为一个共享 Unit；任一子通知失败后保留未完成子集，不丢失 invalidated delta，不重复已接收子项。 |
| 6 | `MU-limit-boards` | first/second/third-plus 共用 `board_notified[session,code]` 与空 code 的 L4。 | 每个 code、session、阶段产生独立 occurrence；阶段升级不被旧阶段 cursor 吞掉，也不因重启倒退到 first。 |
| 7 | `MU-paper-review-daily` | auto/manual/push 路径与 `ReviewScheduleState`、cooldown/L4 并存；PaperReview kind 还有异质 noon producer。 | 以同一个日复盘事实只设置一个物理 owner，先对齐 auto/manual 的 task identity、补推及 exact bytes，再裁决 push 路径旧完成状态。 |
| 保留 | `MU-paper-review-noon` | `NOON_SNAP_LAST` 对应的 noon 结构受时间窗口阻断；冻结目录标 `STARVED`。 | 保持 Starved，记录明确的缺失事实/窗口；不通过补造快照或复用 daily receipt 把 noon 晋级。 |

## 每张实现卡的字段

动手前逐项填写：当前源码 commit；运行制品 SHA；`producer_id` 与 `PushKind`（enum 外写 `none`）；business date/window；source batch 与 freshness；旧/new completion owner；intent key；目标 sink 与接受语义；shadow diff hash；注入的失败点；Draining 回退；真实 receipt id；两个运行日观察结果；旧路径 cleanup commit。以上字段缺任一项，状态保留 `Code Ready` 或 `Shadow Ready`，不能写 `Production Verified`。

来源：[平台路线图 M1–M2](../superpowers/plans/2026-09-28-platform-complete-roadmap.md)、[当晚事实基线](../audits/2026-09-28-platform-m0-baseline.md)。
