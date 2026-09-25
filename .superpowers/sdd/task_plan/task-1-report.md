# Task 1 — NewsAI 独立恢复 owner 与公平人工队列

基线：`903fde985840c56653c786e0c0f73e95f93a6ae8`。范围仅 Task 1；不部署、不重启、不写生产数据库。

## 实施记录

- 已全文读取 brief、AGENTS.md、CLAUDE.md 与 TDD skill 及测试/替身说明。
- 调度采用统一 worker：持久恢复 + 可选的同 tick live 输入，共用 single-flight permit；不会让两个固定顺序 runner 互相饿死。
- 人工项采用现有审计发布 seam 成功后追加确认。不新增外部卡片；发布/ack 失败保留待处理。接受 effect 成功、ack 前崩溃导致的 at-least-once 重复，不提前 ack。
- main 的启动调用、休市常驻 tick、正常 tick 均进入统一 `schedule_tick`。正常 tick 在同批快讯处理后把可选 admitted batch 交给该入口；`raw_batch=None`、空 admitted、selection 未启用、非交易时段不再抑制恢复。
- 恢复分支不持有 analyzer，也不接收 live batch，不创建 assessment；仅加载既有审计及冻结事实/卡片。外发仍经过原有治理与投递闸门。
- 同时存在 live 输入时，每 worker 先分配最多 2 项恢复，再把已消耗数量交给原有候选预算，总工作量不超过 5；仅恢复时最多 5 项。live 遇到已有 assessment 只延期给恢复 owner，不再在同 tick 第二次投递它。
- SQLite 新增 append-only `news_ai_recovery_claim` 与 `news_ai_recovery_review_notified`。按上次访问序号轮转类内项，按最后一次 claim 的类别交替 ready/manual；claim 与扫描放在同一 immediate transaction，失败全部回滚。确认仅抑制该 assessment/原因的人工提示，不修改 assessment 或投递终态。
- 原启动 banner 的“无模型且不可投递则不调度”状态一并更新；即便无这两项能力，人工审计恢复仍有工作可执行。

## RED / GREEN

1. `cargo test --bin monitor br172_scheduler_recovers_without_live_ingress -- --nocapture`
   - RED：session 16009，exit 101，0 passed / 1 failed；失败原因 `persisted recovery must be scheduled without live input`，非编译错误，850 filtered out。
   - GREEN：session 17729，同命令 exit 0，1 passed / 0 failed，850 filtered out。测试使用生产调度 seam，不是源码文本搜索。
2. `cargo test --lib br172_recovery_queue_is_fair_under_a_ready_backlog -- --nocapture`
   - RED：session 15523，exit 101，0 passed / 1 failed；3 tick 实际 manual=0，预期=3，3823 filtered out。夹具为 12 ready + 3 manual，limit=2。
   - GREEN：session 56648，同命令 exit 0，1 passed / 0 failed，3823 filtered out。
3. `cargo test --bin monitor br172_manual_notice_is_confirmed_only_after_publication -- --nocapture`
   - RED：session 22100，exit 101，0 passed / 1 failed；实际事件 `[publish]`、预期 `[publish, confirm]`，851 filtered out。修复为 `publish()?; confirm()`。
   - GREEN：纳入最终 monitor 模块回归 session 30260，exit 0，13 passed / 0 failed。

## 最终限定验证

1. `cargo test --bin monitor news_ai_shadow::tests -- --nocapture`
   - session 30260，exit 0，13 passed / 0 failed / 839 filtered out。
   - 新增的真实调度/并发预算/人工发布确认行为测试全部通过；包括原有相邻回归。部分保留的旧测试使用源码检查，它们不替代新增行为测试的证据。
2. `cargo test --lib database::news_ai::tests -- --nocapture`
   - session 17518 首次在新测试夹具数组转换处遇到 E0282 类型推断错误（未进入测试，不计为行为 RED）；改为取有界结果首项后重新执行。
   - session 70104，exit 0，30 passed / 0 failed / 3796 filtered out。包含真实 SQLite 重开、人工确认写入失败重试、claim 事务回滚、全局 limit、公平性，以及原有 `br172_and_counted_reentry_share_one_physical_delivery` 双账本去重测试。
3. `git diff --check` 通过；最后随本报告一起检查。没有追加重复 check/build/clippy，也没有运行全量测试或 release 构建。
4. 编译仍有仓库既有 warning（lib 131、lib test 67、monitor test 4）；此次验证没有声称消除这些 warning。

## 验收证据索引

| brief 验收 | 生产代码 | 行为证据 |
|---|---|---|
| 无 live、空 admitted、selection disabled、休市仍恢复，且不新分析 | `main.rs:7910/7970/8177`；`news_ai_shadow.rs:39/274` | `br172_scheduler_recovers_without_live_ingress` 四种输入条件均 recovery=1、analysis=0 |
| 单 worker，recovery/live 都获得有界机会 | `news_ai_shadow.rs:39`，候选已有 assessment 只交回恢复 owner | `br172_scheduler_shares_one_worker_without_starving_live_or_recovery`：第二个并发 worker 不启动，同 worker recovery=2 + live=3，全局最多 5；物理去重另由已有双账本行为测试验证 |
| ready/manual 公平且限制总量 | `news_ai.rs:2198/2210` | 12 ready + 3 manual、每 tick limit=2，3 次扫描得到不同 ready=3、manual=3 |
| 发布成功之后才确认，正常 tick 不重复，ack 失败/重开可重试 | `news_ai_shadow.rs:378`；`news_ai.rs:2347/2425` | `br172_manual_notice_is_confirmed_only_after_publication`；`br172_manual_confirmation_and_rotation_survive_reopen_and_failed_ack` |
| 写入失败不能隐式封口 | `news_ai.rs:2198` immediate transaction | `br172_failed_scan_rolls_back_every_claim_and_keeps_global_limit` 第二条 claim 失败时前一条一同回滚；ready/不存在 claim 不能人工确认 |

## 变更文件

- `src/bin/monitor/main.rs`：启动、正常/休市 tick 接线。
- `src/bin/monitor/news_ai_shadow.rs`：统一单 worker 调度 seam、独立恢复、publish/ack、调度行为测试。
- `src/database/news_ai.rs`：持久公平 claim/确认、故障与真实重开测试。
- 本报告。

## 自审与边界

- 没有新增模型调用、外部卡片通道、订单能力、线上修数或部署。
- `limit` 是返回/尝试处理项的全局硬上限，不是全历史完整性校验的 I/O 行数上限；本任务保留原有全链校验。claims 随恢复尝试增长，规模化后的索引/压缩策略属于后续性能工作。
- single-flight 沿用进程内 permit；没有扩大为多进程租约平台。scan claim 是轮转进度，不是排他投递凭证；物理幂等仍由既有双账本负责。
- 普通人工提示确认后不重复；effect 成功、ack 失败/进程退出的窗口允许重发，明确为 at-least-once，不宣称 exactly-once。
- 调度测试使用生产调度 seam，替身仅在 recovery/analysis 边界；SQLite 公平性/确认测试使用真实隔离数据库。现有源码字符串检查不作为这次调度验收的证据。
- 人工发布复用了 `event::publish_delivery`，该方法先持久化并同步审计记录，再返回成功；事件总线无订阅者不影响持久化审计成功。人工确认位于返回成功之后，不以前置确认规避重复。
- 尚未覆盖：线上数据库升级、真实 monitor 运行接管、外部物理通道端到端发送；这些均属于后续授权部署验收，不由单元测试替代。
- 按执行计划/TDD/完成前验证技能，将结论限定在本 brief 和以上最终代码证据内；未派生子代理。

## 交付状态

Task 1 实现与限定回归完成；本报告与 3 个源码文件一并提交。最终 commit SHA 以实现者最终回复及 Git 记录为准。
