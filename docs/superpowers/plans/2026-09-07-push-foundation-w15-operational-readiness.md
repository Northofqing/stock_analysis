# 推送 Foundation W15 运行就绪实施计划

**状态：** Task2B `595f605`、Task3G `ac8a28b` 和 Task3H `a538925` 均已完成限定独立 Spec/quality 审查，无 Critical/Important。Task3H 交付共用原验证内核的采集审计事务适配器，最终相邻回归13/13；Clippy13184为163项既有告警、目标零诊断。182项 Foundation 证据仍对应2B/3G，不冒充3H之后的全仓复验。已关闭切片不重派；可信来源/采集上下文和 W16 部署绑定、认证恢复、同快照 probe 及 W11/W14 联结仍待。W15 尚未完成，逐项证据见 `docs/push-system/implementation-w15-results-2026-09-07.md` §10–12；W16 按独立计划推进无循环依赖的完整读取链路。

**目标：** 交付 Core/Producer/Occurrence 三个范围的就绪判定、可查询的不可变运行快照、绑定前后快照及依赖版本的恢复事件，以及只读部署探针。按 WBS 执行完整 W15，不将单次 source availability 等同于全部就绪能力。

**权威依据：** `docs/push-system/push-system-wbs.v1.json` W15；`docs/push-system/push-system-implementation-rfc.md:993`--`:1047`；`docs/Project_Architecture_Blueprint.md:1398`--`:1453`；W06 catalog、W11 startup recovery 和 W14 phase scheduler。

**执行边界：** 开发只在 `codex/push-reliability-20260905` worktree。生产 monitor 的启动、观察、替换和 owner 晋级不属于本计划；测试使用显式隔离数据。W15 不是 data/provider 许可，也不创建缺失 producer 或把失败空批次改成 NoData。

## 已核对的项目事实

- W06 有 65 个 kind、102 个 producer 和 52 个 Unit；其中 10 个 producer 不属于 MonitorKind 枚举。INACTIVE kind 的 producer_ids 为空，例如 HoldingEvent。因此 inactive 与 producer 缺失不能混算，计数需注明对象口径。
- W06 的 source/schedule/presentation/policy 尚未全部结构化，不能解析自然语言作为能力或版本证据。
- W07 冻结 SQL 没有 readiness snapshot/recovery event 表；现有 Foundation/Event 也没有对应运行时存储。需明确 operational evidence 的存储与查询合同，不能用内存 bool 或日志代替权威。
- W14 已有纯时间判定、原身份与 CAS proposal；ReadyGate、来源恢复证据以及真正的 occurrence 事务仍未接入。
- 旧 BR-246 设计与 `src/bin/monitor/main.rs` 的 diagnostics/readiness loop 已存在，W15 应保留 resident liveness 与数据 readiness 的区别。旧设计文档仍标待复核；具体已实现行为以源码和测试为准。

## Task 1：冻结证据、存储和计数合同

设计产出：`docs/superpowers/specs/2026-09-07-push-foundation-w15-operational-readiness-design.md`。选择独立 operational SQLite，W07 SQL 保持不变；pure assessment 与 attested snapshot 分开，snapshot/recovery 使用无环 hash 和事务 head CAS。

检查 `src/monitor/push_job/{catalog,context,facts,canonical}.rs`、`src/push_foundation/{migration,intent_store,reconciler,phase_scheduler}.rs` 和既有 readiness diagnostics 的实际边界。

产出 `docs/superpowers/specs/2026-09-07-push-foundation-w15-operational-readiness-design.md`（新文件），明确：

- scope、dependency、evidence、snapshot、recovery event 的封闭字段与不可伪造边界；
- Core 的共享 namespace/durable/audit/typed authority/schema/manifest 前提；
- ACTIVE producer 的 source/schedule/presentation/policy、receipt strength、completion policy 和 session 可达性检查；
- registered-contract 的单次 occurrence 输入阻断；
- 唯一 operational snapshot/recovery 存储与原子追加方式、版本、重启读取和损坏拒绝；保持 W07 冻结 artifact 不变；
- snapshot hash 与前后 recovery 引用的无环构造，不能相互递归求 hash；
- 65 kind、102 producer、10 枚举外 producer 的独立完整计数口径；
- 认证恢复来源与 capability/version 变化的证据要求；无证据不得恢复；
- W16 尚未交付的 activation/fence 只作为绑定前提，不伪造生产执行权限。

完成证据：每个 RFC 字段/规则有明确创建者、校验者、存储位置和消费者；选定方案足以执行下面的代码切片，未决技术细节在 Task 1 内核实解决。

## Task 2：实现三范围分类与不可变快照

预计新增 `src/push_foundation/operational_readiness.rs` 与对应 `_tests.rs`，在 `mod.rs` 注册。先通过现有领域类型和小接口实现可观察行为：

| 输入事实 | 状态 | deployment_ready | 处置 |
| --- | --- | --- | --- |
| 所需证据完整且版本匹配 | Ready | true | Continue |
| 共享核心前提缺失 | CoreUnready | false | 启动非零；运行中停止新工作、恢复隔离后受控非零 |
| ACTIVE producer 合同缺失 | ProducerUnready | false | 隔离对应 producer，其他就绪 producer 继续 |
| 已注册合同、已知 occurrence 暂缺证据 | BlockedOnInput | 默认 true | 不执行该 occurrence；不阻断其他就绪范围 |

验证缺失与错误版本、scope/Unit/producer/occurrence 交叉绑定、受影响集合漏项/多项/重复、INACTIVE 零 executable producer、STARVED/OPT-IN 保留既有批准边界。多个故障并存时聚合不能被较低级别的 Ready/Blocked 覆盖。

快照包含 canonical ID、business date、capture time、build、generation、scope/status/reason、依赖和证据引用、受影响集合、recovery 引用、liveness/deployment_ready/exit disposition。错误与 Debug 不暴露 payload、账户、凭据或 protected URI 正文。

## Task 3：持久快照与恢复事件闭环

并行边界：Task 3A 在 `readiness_store_schema.rs` 实现独立 schema/namespace 验证和显式初始化、只读/写入打开；主代理在 `readiness_recovery.rs` 实现无环事件/快照材料、在 `readiness_recovery_codec.rs` 校验重读关联后集成 `readiness_store.rs`。schema 连接与候选事件都不是认证权限，不替代本任务的真实原子提交与恢复验收。

按 Task 1 冻结的存储方案新增 `readiness_store.rs` 及测试（路径可在设计中细化），使用真实隔离存储验证：

- 首次 Pending 事件、snapshot 与查询指针原子提交；
- 恢复事件绑定旧/新 snapshot、旧/新 dependency version、认证来源和时间；
- 先追加已验证恢复证据再发布新 snapshot；不能先把 Ready 暴露给消费者；
- 重启后按 ID 查询同一事实；重复提交幂等，漂移/陈旧版本/并发竞争拒绝；
- 写入或确认丢失时重查原事实，不重复创建恢复事件；
- hash、schema、identity、依赖版本、事件关联损坏失败关闭；
- 无显式 capability/version recovery 时不定时伪造恢复，不把永久缺源变成空批次轮询。

完成证据必须包括重新打开存储、原子失败与重放测试；仅内存测试不证明可查询的跨重启事实。

## Task 4：只读 health/readiness/deploy projection

并行切片 Task 4A：先在 `readiness_probe.rs` 实现纯候选 catalog inventory，覆盖 kind/producer/枚举外 producer 三个分母、INACTIVE/conditional 和 typed 缺项去重；最终接入 Task 3 已验证快照后才能成为权威部署投影。该切片不替代下面的完整验收。子代理仅编辑本模块及其测试；主代理维护领域接口、模块注册、Cargo 调度与提交。

以同一已验证 snapshot 实现查询和机器可读部署输出；如需要独立 CLI，新增显式数据路径的 `src/bin/push_readiness_probe.rs`，不修改 monitor 启动流程。最终文件名在 Task 1 冻结。

输出 snapshot hash、build、generation、状态/原因、受影响 ID、恢复事件和 RFC 要求的计数；清楚区分 kind 与 producer 分母。CLI 的 readiness 退出码按状态判定，liveness 单独输出。错误、缺失或损坏 snapshot 不能默认为 Ready。

验证查询前后存储内容不变、不能调用 provider/sink/转换，并证明 CLI、health 与 readiness 对同一 snapshot 的结果一致。ProducerUnready 的 operational alert 作为可消费的独立事实输出；不新增旁路通知发送。

## Task 5：W11/W14 与来源恢复的联结

已完成只读接口核对（尚未实现联结）：W11 `scheduler_barrier` 只证明固定点遍历完成，不携带 namespace/Unit/generation 等身份；W14 当前没有显式的 input block/recovered proposal 入口；W15 `OccurrenceId` 与 W14 `ScheduleOccurrenceId` 不能比较字符串后视作同一身份。实现需窄化的 schedule identity 绑定视图、保留有效延期窗口的两条受控提案入口，以及经 store 重查的 before/after recovery 关联；不得把 hydrate 用作绕过状态转换的入口。

在同 crate 的 orchestration seam 绑定 W11 恢复完成报告、当前 readiness scope 和 W14 schedule。仅已核验匹配的 ReadyGate 可为新工作提供控制面资格；生产数据仍需实际消费时复验。

验证 Core/ProducerUnready 阻断对应新工作、其他就绪 producer 不受影响、BlockedOnInput 无 provider/prepare；SourceContract 恢复事件须绑定原 occurrence 且在有效窗口内，才可提出 `BlockedOnInput→Eligible`。过期按 W14 policy 处理，Closed/Missed 保持封口，不补造 identity。W16 generation/fence 和业务库原子提交仍须独立实现，不能被 readiness snapshot 替代。

## Task 6：评审、验证与结果文档

按 Spec/Standards 两轴检查所有验收映射、真实持久查询、恢复关联、零副作用查询和 data-permit 边界；有明确回归风险的发现先写失败用例再修复。

至少执行 W15 目标测试、Foundation 与 push_job 相邻测试、实际 probe 的离线验收、rustdoc、cargo check/clippy、相关文档校验和 `git diff --check`。复用仍有效的证据，变更影响到的接口与失败路径必须重验。既有 strict Clippy/catalog 基线分别记录，不能冒充通过。

新增 `docs/push-system/implementation-w15-results-2026-09-07.md`，登记验收矩阵、提交、运行结果和剩余生产边界；更新 `.planning/2026-09-06-push-foundation-runtime/`。W15 通过后按依赖进入 W16，完整 W01--W21/52 Unit 目标保持不变。
