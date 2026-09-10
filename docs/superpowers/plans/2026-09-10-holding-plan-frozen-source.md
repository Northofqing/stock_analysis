# 持仓计划：同一快照与行情批次来源接线

状态：Task 1 已实现、验证并经独立审查通过，源码 `112ff8f9d15834a4a29de5439909f01413d08ad4`；27 项定向测试与 scoped format/Clippy 通过，保留既有告警 Minor。基线 `ca902d8112db187faa56b2f87138f22e6d12e808`。结果见[实施记录](../../push-system/implementation-holding-plan-frozen-source-2026-09-10.md)，不代表完整迁移或生产验收。

## Outcome and Spec

把已确认的持仓重复读取和行情证据丢失问题修到真实准备入口。手动与周期入口继续调用同一个 `prepare_holding_plan_messages`；一次准备只读取一份用户确认快照，按其代码请求完整 `TopStockBatch`，将快照身份和原始行情证据写入交给 counted delivery 的 canonical bytes。

绑定需求来自 [持仓调用链](../../push-system/holding-plan-call-chain-2026-09-10.md) 的来源缺口和 [RFC MU-holding-plan](../../push-system/push-system-implementation-rfc.md#mu-holding-plan--持仓计划)。已核对当前 `main.rs` 的准备函数、`market_data.rs::fetch_realtime_quote_batch`、用户快照模型和 `CountedDeliveryBinding`。本切片不把来源保留称为来源认证或完整 Foundation 接管。

## Global Constraints

- 所有项目工作限于 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`。不读取实际 `.env`、生产数据库或 monitor 进程，不执行 provider、LLM、通知、订单、远端 Git、部署或 CI。
- 只改 `src/bin/monitor/main.rs` 的持仓准备接线/直接相关注释，新增 `src/bin/monitor/holding_plan.rs`。不改共享 `market_data.rs`、模板、账户健康、策略、数据库 schema、日表和 durable 恢复逻辑。
- 一次准备读取一份快照；代码只来自该快照，不重读最新快照、不回退本地持仓代码；行情使用已有 `fetch_realtime_quote_batch`，其原始 `BatchEvidence` 不得被丢弃、补造或用本地时钟替代。
- 保持空持仓静默、缺行情逐票跳过、非法非正成本逐票跳过；保持现有 >5 / <-3 / 其余的 Reduce/Add/Hold、文本模板参数、数量转换、Ticket scope、InternalDurable origin、retry_authorized=true，以及 `holding-plan:{date}:{code}`。
- 同一轮仅捕获一次本地准备时间；业务日、正文 HH:MM、canonical 的本地 observed_at 都来自它。行情 observed_at/source_at 分别保留，缺失 source_at 保留为 null，不能提升为已认证时间。
- 不改新鲜度政策、有效修订判定、Rolling 1800 秒、日预算、日表过滤、发送资格或启动恢复；不回写旧 binding。新来源字节会改变新 decision 的 source/subject hash，这不是新的重发许可，上线前仍需迁移门禁。
- 父 agent 负责唯一 Cargo 队列、Git 和公开文档；实现 agent 是唯一 Rust 写入者，不自行运行 Cargo 或提交，不派生 agent。测试仅使用固定时间、内存快照/行情 fixture 和真实 renderer/binding，不能调用生产 adapter。

## Task 1: 接通持仓冻结来源并验证实际提案

### Files and interface

- 修改 `src/bin/monitor/main.rs`：声明新 holding_plan 模块；保留现有 async 准备函数供两条调用路径使用，使其成为薄的真实适配接线。原准备循环移入新模块一次，不保留复制版本。`PreparedHoldingPlan` 可以原位复用，保持调用方不变。
- 新建 `src/bin/monitor/holding_plan.rs`：实现完整准备操作及其离线测试。使用小的、领域明确的可注入数据库/行情/时钟 seam；生产和测试必须走同一准备操作及真实渲染/binding。不得只测试一个生产不调用的 pure helper，也不得新建泛化能力框架。
- 一次捕获时钟并读快照；明确缺失和读取失败错误。空快照不取行情。有持仓时用快照的精确代码请求一次已有行情 batch adapter。用该快照的价格/数量与该 batch 的现价生成完整提案。
- 每票 canonical 保留旧 code/intent/price/cost/quantity/pnl_pct/observed_at，并增加 `schema_version: "HOLDING_PLAN_SOURCE_BINDING_V1"`、`snapshot`、`quote_batch`、`requested_codes`。`snapshot` 保存 snapshot_row_id、snapshot_id、effective_at、confirmed_at、source、confirm_empty、evidence_sha256；该票名称也要绑定（原文渲染用了它）。`quote_batch` 完整保存 provider（已有枚举 wire 名）、source、source_at、observed_at、batch_id。复用现有 SHA/binding 构造，不新增身份权威。

### Red → Green and acceptance

1. 先形成可离线调用、真实生产接线会使用的准备操作，保留原来源 canonical，写一个检查返回 binding 内快照/行情来源的失败测试。交父 agent 实际运行、记录编译成功后的断言失败；不要把缺符号的编译错误当 RED。实现 agent 在每次验证期间冻结 Rust，等待父 agent 结果。
2. 修复生产精确代码采集及 canonical 来源保留，通过同一测试。随后补齐同一操作的回归：快照缺失/读取失败不取行情；确认空/无项目静默；行情错误传播；部分覆盖只产出有行情票；两票 Reduce/Add 及 Hold 固定示例、模板关键价格/数量；同一轮日期/时分/本地 observed_at 一致；source_at=None 不补值；仅快照身份或行情批次身份变化能在返回的 source bytes/fingerprint 中被看见。
3. 快照 A/B 变化用受控数据库 seam 验证：返回 A 后外部最新值变成 B，最终请求与提案仍属于 A。测试观察返回的真实正文/binding，代码请求记录只作为数据库/网络系统边界的辅助证据。不得用函数名或源码字符串断言代替行为。
4. 保留当前非正成本/缺票行为。不要顺带添加新鲜度、市场身份或业务修订的新政策；发现的已有问题写进报告。
5. 父 agent 审计实际测试路径后运行新模块全组、原手动入口回归和已有纯行情投影测试；运行两份改动文件 scoped rustfmt、diff check、monitor Clippy 并比较基线告警。全仓 fmt 的已知六文件旧差异保留，不借此修无关文件。所有 Cargo 用 --offline，不运行完整未审计 monitor/lib suite。
6. 实现 agent 写 task-1-report.md，包含实际 RED/GREEN/最终证据、文件范围、自检和剩余边界。父 agent 单独提交源码，生成 BASE..SOURCE review package，安排独立 Spec/Quality 审查；有重要问题交原实现 agent 修复并定向复审。
7. 父 agent 在 docs/push-system 写实施记录，更新 README 与持仓调用链及本计划状态；记录来源形状兼容影响、精确源码身份/测试结果/未完成项。不得将此次切片计为 Unit 迁移完成。

## Remaining full objective and rollout

该切片之后仍要完成：有效业务修订与再次发送资格、日表与 durable 的统一持久完成权、来源认证/新鲜度治理、真实 Foundation 业务接线、52 Unit 六门禁及独立生产窗口验收。启动只恢复原 immutable bytes/decision，不能以新来源重建旧载荷。本计划不安排生产切换；回退代码不能假定已经生成的新 decision 与旧 source hash 相同。
