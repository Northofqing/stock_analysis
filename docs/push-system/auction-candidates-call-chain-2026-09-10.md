# 集合竞价候选单元：实际调用链与剩余改造证据

日期：2026-09-10。源码锚点：`409dbaff2303fc9c3d6bcbec9d31ee1e9f95f3f8`。范围为 `MU-auction-candidates` 的 A-02 竞价重推、P-05 候选台、T-08 候选失效三个 producer；只读核对代码，没有读取真实 data、数据库、环境变量或进程，没有发送、测试运行或生产复现。后续相关源码改变时应重新核对引用，不把此快照冒充最新部署。

目标是补齐尚未逐链核对的一个 Unit，区分实际业务入口、源批次、发送结果、游标和效果顺序；不是实施计划，不新增激活权威，也不改变冻结能力目录。

## 1. 实际调度与发送边界

| 阶段 | 当前行为 | 源码证据 |
| --- | --- | --- |
| Unit 归属 | 三个 producer 共属 MU-auction-candidates，主时段集合竞价；历史目录指出共享非原子推进链 | [冻结目录](push-capability-catalog.v1.json#L8829)；仅取归属，不把旧摘要当最新接线 |
| 外层资格 | market_loop 先检查交易日、等待 market active，再进入本轮监控；初始化内存完成标志为 false | [main.rs](../../src/bin/monitor/main.rs#L9435)、[标志初始化](../../src/bin/monitor/main.rs#L9574) |
| 时间窗口 | 日历 Auction 为 09:15（含）至09:25（不含）；进入本分支再要求当前时间不早于09:20。它是分支进入条件，不是后续物理发送截止时间保证 | [日历常量](../../src/calendar.rs#L24)、[日历分支](../../src/calendar.rs#L522)、[main 时段条件](../../src/bin/monitor/main.rs#L9695) |
| 主调用 | 完成标志为 false 时，先 await A-02，再 await P-05；只有本次两个 bool 同时为 true 才关闭本轮标志 | [main.rs](../../src/bin/monitor/main.rs#L9762) |
| 扫描间隔 | 完成该轮竞价分支及其他工作后 sleep 30秒，再 continue；不是固定30秒内必达，前序采集/处理耗时也会增加间隔 | [main.rs](../../src/bin/monitor/main.rs#L10020) |
| 展示入口 | A-02/P-05/T-08 分别获取自身注册的 family/kind/producer/renderer token；未注册则 Denied，不会退回无类型发送 | [宏](../../src/bin/monitor/push_templates.rs#L37)、[A-02/P-05 注册](../../src/bin/monitor/presentation_registry.rs#L213)、[T-08 注册](../../src/bin/monitor/presentation_registry.rs#L111) |
| 结果语义 | 三条 dispatcher 最终均以 PushOutcome.is_pushed 转 bool；该方法只有 Pushed 才为 true，Deduped 不算成功 | [A-02](../../src/bin/monitor/push_templates.rs#L7503)、[P-05](../../src/bin/monitor/push_templates.rs#L8093)、[T-08](../../src/bin/monitor/push_templates.rs#L14579)、[is_pushed](../../src/bin/monitor/notify.rs#L2199) |

这里只证明真实源码会进入现有展示/治理入口，不证明本机已经执行、接收端已收到，或新 Foundation 已接管。

## 2. 数据与业务规则

A-02 在 [dispatch_auction_repush](../../src/bin/monitor/push_templates.rs#L7459) 自己调用 load_real_candidate_batch；源失败/无候选均 false。它排除现价缺失、非有限或不大于0，以及热度缺失/非有限的候选；先 Strong 档优先，再热度降序，取前5。全部被排除也 false；非空才渲染并交注册入口。明确价格/热度过滤、排序和 Top5 都属于迁移必须保留的旧规则，不是本次新策略。

P-05 在 [dispatch_candidate_board](../../src/bin/monitor/push_templates.rs#L8029) **再次调用**同名 loader，不复用 A-02 已捕获的 batch；源失败/原候选为空即 false。它使用全部 batch.entries 构建当前 code 集和候选台，不沿用 A-02 的 Top5。两者“来源路径相同”不等于“同一次读取、相同观察时间或相同候选材料”。

共享 [load_real_candidate_batch](../../src/bin/monitor/push_templates.rs#L3916) 会先加载来源上下文；空 entries 直接返回空批次且没有行情/统计证据。非空则并行获取实时行情和市场统计、检查统计投影、再组装；[RealCandidateBatch](../../src/bin/monitor/push_templates.rs#L3688) 持有两种来源证据。A-02直接取entries，P-05也没有将两种证据与后续消息/快照/预测写入绑定为独立持久完成记录。这是接线缺口，不表示来源从未记录日志，也不表示可以跨源补造字段。

## 3. P-05 的实际效果顺序

所有动作在主卡发送之前发生，具体次序来自 [push_templates.rs:8042](../../src/bin/monitor/push_templates.rs#L8042)：

1. 从本批 entries 构造 codes_now；读取日期 JSONL 的最后一行作为 previous。
2. previous−codes_now 逐票调用 T-08，忽略返回 bool。名称在当前集合查找，而该 code 已不在当前集合，因此这条差集路径会使用 code 作为名称后备值。
3. Strong 且 current_price 为 Some 的候选发起 prediction_tracker 写入；heat_score 缺失时用50，目标日期是当前本地日期加5个日历日。这里没有 A-02 同样的现价有限/正值过滤。每条数据库写入结果和后台任务结果均忽略。代码证明重复调用风险，不足以证明数据库最终必然存在重复行。
4. 将当前 code 集追加到日期 JSONL，之后才渲染 P-05主卡并尝试发送，返回主卡自身的 is_pushed。

[快照读取](../../src/bin/monitor/push_templates.rs#L8002) 将文件读取失败、没有最后一行、JSON解析失败统一折为 None；[快照写入](../../src/bin/monitor/push_templates.rs#L8010) 忽略建目录、打开、追加和序列化失败，没有向调用者返回持久化结果。本文只读了这些函数的代码，没有访问它们命名的真实目录。

## 4. 可由源码推出的失败序列

### 4.1 两路分时成功仍不能关闭共同标志

三个 kind 都不在当前 counted 映射内：[is_counted_kind](../../src/bin/monitor/durable_delivery_runtime.rs#L1126)、[穷尽匹配与默认None](../../src/bin/monitor/durable_delivery_runtime.rs#L2274)；且均为非Emergency的Important级别：[级别映射](../../src/bin/monitor/notify.rs#L310)。因此成功后会记录进程内冷却；命中冷却返回 Deduped：[冷却表与判断](../../src/bin/monitor/push_templates.rs#L14797)、[发送后的记录](../../src/bin/monitor/push_templates.rs#L14908)。A-02冷却600秒，P-05走默认1800秒：[配置](../../src/bin/monitor/notify.rs#L418)、[默认分支](../../src/bin/monitor/notify.rs#L459)。

下表是同一进程、时钟正常且冷却尚未过期的**逻辑反例**，不是实际生产日志或已运行测试：

| 尝试 | A-02结果 | P-05结果 | repushed && board_pushed |
| --- | --- | --- | --- |
| 第1轮 | Pushed，记冷却 | 源失败或发送失败 | false |
| 第2轮 | Deduped → false | Pushed，记冷却 | false |
| 后续轮 | Deduped → false | Deduped → false | false |

两路即使已各成功一次，仍没有同一轮同时true；两种冷却都长于5分钟竞价窗口，在上述条件下标志不会于该窗口关闭。loader、P-05快照/预测/失效路径位于主卡冷却判断之前，可能继续被调用。不能直接把任意 Deduped 当完成来修：冷却命中不是精确请求的持久接收证据。

### 4.2 失效通知失败后，差集线索先被推进

假设旧快照为 `{A,B}`，新候选为 `{A}`，T-08(B)返回false；若随后快照追加成功，就把最后一行推进为 `{A}`，不论主卡是否成功。下一轮当前候选仍为 `{A}` 时，差集为空，不再因这条旧差集尝试B。依据是 [忽略T-08返回](../../src/bin/monitor/push_templates.rs#L8057)和[先写快照再发送主卡](../../src/bin/monitor/push_templates.rs#L8090)。反之，快照写失败也没有显式状态告知调用方，不能从主卡true推定快照已持久化。

### 4.3 当前集合为空不会发“全部失效”

P-05在读取旧快照前就对 entries.is_empty 返回false（[8039行](../../src/bin/monitor/push_templates.rs#L8039)），所以旧集合非空、新集合为空时不进入差集。应由真实来源证据和业务政策区分已验证清空与暂缺数据；不能直接删掉空集门禁，将来源故障解释为所有候选失效。

### 4.4 分支时间条件不是发送截止约束

进入竞价分支后先 await P-02采集和分发，再执行 A-02/P-05各自的异步采集；[main.rs:9763](../../src/bin/monitor/main.rs#L9763)只是重新生成文案时间，没有在两路调用前重新检查session或09:25截止。由此可推导耗时跨越截止点的可能性；本次未测量延迟，不能声称实际越界发送已经发生。后续调度必须明确“进入窗口即可完成”还是“截止后禁止新发送”，并通过真实时钟/请求绑定验证，不沿用注释当作deadline证明。

## 5. 对后续方案的约束与状态

已完成本次源码路径核对及反例推导；未做修复、运行期RED、迁移认证或生产验证。它补足第7个可指出实际旧业务入口的 Unit，剩余45个尚未逐项追完；7/52不是迁移完成率。

后续需在独立实施计划中处理：一次捕获并明确A-02与P-05不同投影；按子动作保留精确成功/失败/未决证据；将主卡、失效通知、快照和预测写入作为不同效果纳管；明确快照代表“已观察集合”还是“已通知集合”，不能混用同一游标。任何重试必须保留真实未决状态，不能用进程bool或Deduped伪造receipt。来源为空、IO失败及观察时点需要结构化保存。

W18严格请求入口已在9f35ab1完成限定复审，已准备的 [P-02量能选集诊断计划](../superpowers/plans/2026-09-10-auction-selection-diagnostics.md)是接下来的实施任务；本文件不插入第二个Rust作者，也不将新发现混入P-02已确定范围。完整W17/四actor共同fence/真实来源授权和52Unit上线门禁仍未由本次核对关闭。
