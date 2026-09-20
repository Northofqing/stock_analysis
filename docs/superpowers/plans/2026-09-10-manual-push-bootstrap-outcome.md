# 手动推送：真实健康准备与批次失败退出

基线：`43f48933de599acd9c8207d72d6610ded2a08ba4`。本任务落实[持仓计划调用链核对](../../push-system/holding-plan-call-chain-2026-09-10.md)发现的手动入口问题；它是整体迁移的一个修复切片，不是 52 个 Unit 的迁移验收。

交付状态：Task 1 已完成，初版源码 de990c8、最终源码 41e7762；两次真实 RED 后完成修复，初审 Important 经一轮 date-only seam 修正和限定复审关闭。修后 10 项 runner 回归、scoped fmt/Clippy 通过；未改 scheduler 的 7 项证据沿用。全仓 fmt 的 6 文件旧差异及生产验收边界保留，不标整体目标完成。

## Global Constraints

- 唯一工作目录为 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`；保留其他改动，不访问根 checkout 或其他计划私有目录。
- 不运行生产 monitor（包括读取进程、启动、停止）、实际 `.env`、实际数据库、行情、provider、LLM、sink、订单、凭据、网络或部署；只运行审过副作用的定向离线测试。
- 不改变 `OpportunitySchedule::push_window` 的时间选择合同。盘前 P-01 仍由常驻 scheduler 或专属 `--compensate=P-01` 拥有。盘中顺序保持 I-01、I-02、I-03、D-01、I-04；Evening 与 Outside 保持 A-01、A-10。
- 依赖 banner 的手动 dispatcher 在执行前准备真实健康状态，复用 `refresh_banner_state`，不得伪造 Normal/Full/零值，不得为了填 banner 调用会发送状态通知的 `evaluate_account_mode_hook`。盘中健康失败拒绝盘中批；Evening/Outside 健康失败只阻断 A-01，A-10 仍继续执行。
- 各 dispatcher 的既有成功/未确认定义保持不变；失败不自动重试、不撤回已成功效果。HoldingPlan 的来源、阈值、occurrence、冷却、daily 表与恢复合同本轮不变。
- 批次存在任何未确认或拒绝时返回 `Err`，使已有 CLI 错误分支以 2 退出；仅全部确认时返回 `Ok` 并以 0 退出。保留具体失败项。无数据仍沿用现有 dispatcher 的返回合同，不在本轮重新定义成功。
- 不改变 `--test`、`--push-dry-run`、`--review`、e2e、compensation、普通 startup recovery 的入口行为；不把离线 runner 测试称为生产 CLI 端到端验证。
- Rust 只有一位实现 agent 写入；父 agent 负责唯一 Cargo 队列、证据捕获、Git 和公开文档。实现与审查 agent 不派子 agent。

## 已核对的现状与范围

`main.rs::run_daily_pushes` 先读取未初始化的 `LATEST_BANNER`，但正常服务的 account hook 在 CLI 分支之后。该函数收集失败后无条件 `Ok(())`，已有主入口实际根据 Result 选 2/0。A-01 的 generic gate 使用 CombinedAccount，`v14_adapter::governance_ctx_for_banner_at` 读取相同 banner；A-10 的 counted gate 使用 CountedSourceOnly，不读取 banner。盘后不能增加 A-10 对账户健康的依赖。

现有 `review_batch` 属于复盘的 typed outcome、日历、schedule 与审计合同，不能为这个小修复复用其业务调度状态机。以专属 `manual_push` module 集中现有手动编排；真实外部采集/投递的 adapter 与隔离内存 adapter 接在同一个内部 seam，生产调用和测试运行同一份编排。

窗口特别限制：当前 Intraday 判断以 `NaiveTime` 全等匹配配置时间，真实时钟带纳秒通常命中 Outside。此为另一个需核对既有时间合同的问题，本轮不悄悄改成整分钟、全天盘中或自动补偿。

## Task 1: 修复实际手动 runner 并完成定向回归

### 文件与接口

- 修改 `src/bin/monitor/main.rs`；新增 `src/bin/monitor/manual_push.rs`（实现与该 module 测试）；不修改 dispatcher、scheduler、durable、Cargo 或业务 schema。
- `run_daily_pushes()` 仍是实际 CLI 调用函数，捕获一次真实时间，调用新的手动 runner 并直接传回它的 Result。日期/HHMM/窗口均来自该次捕获。CLI 的现有 2/0 与 JSONL flush 退出机制保持。
- 新 module 只负责窗口编排、健康准备、失败聚合。内部 effect adapter 采用有业务含义的方法（refresh/read banner、四个盘中 dispatcher、HoldingPlan 整批投递、两个盘后 dispatcher），不用任意字符串 endpoint、通用插件框架或可公开配置的能力注册器。
- 真实 adapter 转发到既有真实函数；HoldingPlan 的 token、逐项 counted 投递、失败明细与空批处理从旧入口移动一次，不复制第二份实现。内存 adapter 的状态和结果均为测试自有，不使用全局 banner/test mode、真实 DB、provider 或 notify。
- 允许先做保持行为的 seam 提取，保留原本“未 refresh”与“失败后 Ok”两处行为，以便真实 RED 能检测它们；此准备不是修复，也不得把编译失败当 RED。提取后只保留一份生产使用的编排。

### 逐切片验证

1. 完成保持行为的提取与一个已有 banner 的正常批次基线测试，确认真实 CLI 转发同一 runner。父 agent 运行 `cargo test --offline --bin monitor manual_push::tests::`；实现 agent 在父 agent 确认运行结束前不改源码。
2. 新增初始无 banner、只有 health refresh 能提供保守 banner 的回归；测试通过 runner 观察实际投递请求携带该状态，修复前应真实失败。父 agent 取 RED，随后实现：Intraday 在首个 dispatcher 之前 refresh 并读 banner，失败拒绝盘中批；Evening/Outside 在 A-01 之前 refresh 并读 banner，失败记录 A-01 健康失败并跳过 A-01，但继续 A-10。不能使用陈旧缓存继续受影响的发送。Preopen 不做新增健康或投递效果。再取得 GREEN。
3. 再新增部分未确认而其他投递仍成功的回归，断言 runner 返回带原失败项的 Err、已成功项不重发、余下项仍按既有顺序处理；取第二个 RED 后修复批尾返回，再取 GREEN。
4. 补齐覆盖：全部确认成功；多项失败保留全部标签/明细；HoldingPlan 失败明细保留；Evening、Outside 的健康准备和 A-01/A-10 路由；盘后 refresh 失败或 read banner 失败均保留 A-10 执行且批次 Err；Preopen 拒绝且无新增健康/投递效果；盘中 refresh 失败以及成功 refresh 后仍无 banner 的拒绝；同一捕获日期/HHMM。不把 false/NoData 改成送达，也不测试或修改新业务去重策略。
5. 最终运行上述 module 全部测试与现有 `cargo test --offline --lib opportunity::scheduler::tests::`（测试均为纯内存/时间计算）。静态检查 `cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --offline --bin monitor --message-format=json`。警告必须如实记录并比较本任务基线，不伪称全仓无警告。若任务引入新 warning，应修复后跑受影响检查。

父 agent 单独执行命令并记录真实退出状态；每次等待使用工具实际返回的 session ID，不猜测。最终定向测试前后核对修改源码的 SHA，验证期间禁止并行 Rust 改写。若编译/测试有失败，先分类是任务缺陷、环境限制还是既有问题，不重复无效命令。

### 完成证据

实现 agent 报告包含移动与修改的准确范围、两个 RED 和最终 GREEN 的命令/日志路径/退出状态、测试用例到合同的对应、真实 adapter 的静态接线证据、剩余限制。父 agent 固定 BASE..SOURCE 提供独立 Spec 与 Quality 任务审查。审查只复用有效证据，不重跑同版 Cargo。

完成后父 agent 在 `docs/push-system/` 写中文实施记录，更新 README 与剩余证据记录，并把原调用链报告的基线问题标成“历史证据，本项另有修复”，保留其他未完成项。离线验证不证明真实初始化输入可用、真实发送成功、完整迁移完成或现有生产进程已使用新代码。

## 后续与回退

HoldingPlan 同批持仓/行情证据、每日完成权、启动恢复时窗，以及实际 scheduler 时钟粒度仍为后续工作。此项不修改持久化格式，若需撤销可用独立源码提交作可审查的 revert；不替用户合并、部署或回退生产。
