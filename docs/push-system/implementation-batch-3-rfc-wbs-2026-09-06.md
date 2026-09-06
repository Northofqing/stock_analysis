# 第三批：推送实施 RFC、持久化协议与精确 WBS

> 执行要求：使用 `subagent-driven-development`，每个任务依次经历实现、规格复核、质量复核和提交。勾选完成只表示第三批文档规格交付，不表示运行时已改造、生产已晋级或用户已收到消息。

**目标：** 把已批准的 108 项裁决、当前架构蓝图、最近五日生产证据和第二批 65-kind/102-producer/52-Unit 目录收敛为一份可执行实施 RFC；精确定义类型、SQLite DDL、状态转换、跨库崩溃恢复、运行门禁、52 个原子迁移单元的估算/依赖/晋级顺序。

**架构：** RFC 是拟议实现的规范说明；独立 SQL 是可执行存储合同；WBS JSON 是估算和依赖的唯一机器事实源；Markdown 中的 WBS 摘要由 JSON 确定性生成。RFC 不复制现有 durable receipt，也不制造跨业务库与 durable 库的假原子事务：业务库保存 intent 和仅追加 transition，durable 库保存交付决定/attempt/receipt，finalizer 只消费重新校验的终态引用。

**技术栈：** Markdown、JSON、SQLite 3、Ruby 2.6.10 标准库、现有 `scripts/architecture-docs/` 校验接口、Git。第三批不修改 Rust、Cargo、配置、数据库、CI 或真实推送。

**规格依据：** `grill-decisions-2026-09-02.md` Q1--Q108、`push-capability-catalog.v1.json`、`push-evidence-manifest.v1.json`、本批纳管的架构蓝图与最近推送证据。

## 全局约束

- 只在隔离分支 `codex/push-reliability-20260905` 实施；起点 HEAD 为 `288e8b2`，运行时 Rust/Cargo 仍以 `07781bf386aafdf202851ae928efee8920387058` 为审计基线。
- 原工作区有 160 个 unmerged path；只从其读取本计划列出的八份文档输入，不修改、暂存、解决冲突或提交原工作区内容。
- 导入文件必须逐字节等于原工作区输入；复制前后都校验尺寸和 SHA-256。校验器读取路径必须限制在给定 root 内，并拒绝缺失、重复、越界、符号链接和哈希漂移。
- 不读取新的生产数据库、日志或账号数据，不触发 provider、LLM、sink、webhook 或消息发送。DDL 只在测试临时目录中新建 SQLite 数据库。
- 四时段是 Epic，不是切换原子单元。52 个 MigrationUnit 必须逐项引用第二批目录中的稳定 Unit ID；不得按 kind 数量重造另一套迁移边界。
- 现有 `durable_delivery` 只有 23 个 counted kind、14 态和 `Accepted/Rejected/Uncertain` transport authority。RFC 必须显式定义它与 65 个业务 kind 的适配边界，不能声称当前已经全覆盖。
- v18 只复用 Decision/Data/Paper/Audit 的 typed reference；v19 只复用 ReasonCode、指标和隔离的最小交集。Quiet/Banner/breaker 仍属后续平台设计；v20 不进入本批实现范围。
- 所有第三批制品保持 `PROVISIONAL`。文档的 `Implementation-Ready` 只要求干净 Git 基线、完整 RFC/SQL/WBS、蓝图去拟议化、两份离线 HTML 和统一 checker/CI 全部门禁通过；它不要求运行时已经实施。运行时另按 `Foundation Ready`、`P0 Production Verified`、`Architecture Release Candidate`、`Program Production Verified` 四个里程碑独立判断，不能用文档状态冒充生产完成。
- 所有 Ruby 兼容 2.6.10，只使用标准库，不使用 `filter_map`、`tally` 或网络依赖。修改使用 `apply_patch`；八份大文件导入属于字节级机械复制，但复制后必须由 SHA 门禁证明无改写。
- 每个任务只提交列明文件。审查发现由原实现者修复；controller 不直接代修。每个提交后工作树必须干净。

## 已冻结的八份 RFC 输入

| 路径 | 字节 | SHA-256 | 角色 |
| --- | ---: | --- | --- |
| `docs/Project_Architecture_Blueprint.md` | 165130 | `a1acf98ec960880934285d1a71ecf6fba068d809b08174ab51871a91645f2f75` | 当前架构与 §24/§25 拟议设计来源；Markdown 为语义源 |
| `docs/Project_Architecture_Blueprint.html` | 540856 | `4c4b8b8a34206dc7cde66c4a7c6e330679bb2cdba7341b4ee54e83d494a80107` | 用户明确指定的可视化快照；不是独立语义真相 |
| `docs/push-system/comprehensive-reanalysis-2026-09-05.md` | 47302 | `46b644f7e17fa0854276a928ae1e4f70b918b5f973fd16e5267d77b37d73abbd` | 08-31--09-04 生产证据分析、风险排序和验收样本 |
| `docs/push-system/recent-push-evidence-2026-09-05.json` | 405754 | `0d10d3ad158378c6c34ddab4ab9d074019f78e956248e1c7830e7224c909ed54` | 脱敏聚合、身份和对账数据；不等于实时生产状态 |
| `docs/push-system/all-push-kinds-2026-09-05.md` | 27905 | `50ee2e5fa63b9584475e7a471b1ea0646e81e47335e2960be605b7acafca3390` | 原工作区 67-kind 历史快照；与隔离基线 65-kind 冲突时仅作差异证据 |
| `docs/push-system/push-documentation-hardening-plan.md` | 9426 | `752024603b5cb67e031a433f5a3496bb7ea567e91e8bece36061f715c6197fef` | 任务 2 原始验收要求和后续 HTML/CI 边界 |
| `docs/push-system/reanalyse-recent-pushes.rb` | 20408 | `1f4b5dbf2efeef914a083f52f4963661bad8bc7b11fe66f145bdbaf820cc080a` | 最近证据派生程序快照；本批不重新读取生产数据 |
| `docs/push-system/verify-reanalysis.rb` | 4230 | `c535ff7c2ff7864cf98fba8ab139f247cdde4701a99c1956836518735a6938ec` | 最近证据内部一致性校验快照 |

65 与 67 的冲突采用“同一来源基线内闭合”规则：正式 RFC 的实施范围以隔离分支 `07781bf` 的 65-kind、102 producer、52 Unit 为准；67-kind 报告中的 PaperBuy/Watchdog 仅记录为原冲突工作区新增候选，不能获得本分支代码证据、Unit 或激活资格。

## 深模块接口边界

```text
PhaseScheduler / manual command
        |
        v
prepare(RunContext) -> PreparedFacts -> project() -> JobDecision
                                              |
                                     Ready(PreparedPush)
                                              |
               business DB intent + transition/outbox
                                              |
                                              v
             DeliveryCoordinator -> transport authority
                                              |
                         VerifiedTerminalRef only
                                              |
                                              v
       Finalizer(CompletionPolicy) -> business completion transition
```

- `prepare` 只取一次数据并冻结 source references；shadow 和 active 共享同一 `PreparedFacts`。
- `project` 产生可比较的 typed decision；`PreparedPush` 绑定 exact rendered bytes、semantic projection、identity 和 hash。
- intent identity 不含 payload hash；同一 intent 的 payload/evidence hash 漂移进入 `ResolutionRequired`。
- coordinator 隐藏 lease/fence/transport 重试；对应用层只返回可重新验证的引用或稳定 ReasonCode。
- finalizer 是唯一能依据 `CompletionPolicy` 推进业务通知完成状态的模块；schedule occurrence 关闭和通知游标推进分开记录。
- P01/N02 可有专用 authority adapter，但必须满足同一个应用结果合同，不能另建第三套完成真相。

## Task 1：不可变 RFC 输入与 SHA 门禁

**文件：**

- 修改：`.gitignore`
- 新增：`docs/push-system/README.md`
- 新增：上表八份输入
- 新增：`docs/push-system/rfc-input-manifest.v1.json`
- 新增：`scripts/architecture-docs/rfc_inputs.rb`
- 新增：`scripts/architecture-docs/check-rfc-inputs.rb`
- 新增：`scripts/architecture-docs/test/rfc_inputs_test.rb`

**接口：** `ArchitectureDocs::RfcInputs.validate(root)` 返回稳定错误码字符串数组；CLI 为 `check-rfc-inputs.rb --root ROOT`，成功 exit 0、内容失败 exit 1、参数失败 exit 2，永不写输入。

- [x] 在 `.gitignore` 精确放行八份输入、本批计划、manifest、RFC、SQL、WBS 和结果文件，不放开整个 `docs/`。
- [x] 复制前校验原工作区八份输入的路径、普通文件身份、字节数和 SHA；机械导入后逐项验证相同。不得以换行规范化、format 或重新生成替代原字节。
- [x] manifest 顶层固定 `schema_version:1`、`status:"PROVISIONAL"`、`source_workspace:"root-worktree-snapshot"`、`captured_date:"2026-09-06"`、`inputs`，不得保存机器绝对路径。每项固定 `id/path/bytes/sha256/role/authority/conflicts`；HTML 的 authority 明确为 derived visual snapshot，67-kind 的冲突明确为 non-baseline evidence。
- [x] 先用公开 CLI 写 RED 测试：成功、缺失、字节变化、尺寸变化、重复 id/path、非法状态/schema、绝对/越界路径、普通文件被 symlink 替代、未知参数。
- [x] 实现最小 validator；读取前 realpath containment，拒绝目录和链接；使用二进制字节算哈希。
- [x] 运行输入与来源测试、真实 CLI、五个 Ruby 语法检查和 hand-authored diff check；全部通过。八份不可变输入原有 11 处 blank-at-EOL 由 size/SHA 管理，全 staged diff 仅用命令级 `core.whitespace=-blank-at-eol` 检查且不改 Git 配置。
- [x] 独立规格/质量复核八项路径/尺寸/SHA/角色、65/67 裁决、CLI 和路径安全；最终 Critical/Important/Minor 均为 0。提交 `bb3eccd`（`docs: freeze batch 3 RFC inputs`）。

## Task 2：RFC 领域合同、应用结果与 ReasonCode

**文件：**

- 新增：`docs/push-system/push-system-implementation-rfc.md`
- 新增：`scripts/architecture-docs/rfc_spec.rb`
- 新增：`scripts/architecture-docs/check-rfc.rb`
- 新增：`scripts/architecture-docs/test/rfc_spec_test.rb`

**接口：** `ArchitectureDocs::RfcSpec.validate(root)` 返回稳定错误数组；`check-rfc.rb --root ROOT --draft|--check` 为只读。`--draft` 只豁免 PROVISIONAL/未发布状态，不豁免类型、枚举、映射、交叉引用或哈希缺失。

- [x] 先写 RFC 导航、规范词义、范围/非目标、Foundation→Unit→tail cleanup 拓扑，以及 CURRENT/PROPOSED/PROVISIONAL 的边界。
- [x] 精确定义字段、构造不变量和所有 variant：`RunContext`、`PreparedFacts`、`SemanticProjection`、`PreparedPush`、`JobDecision`、`VerifiedTerminalRef`、`CompatibilityEvidenceRef`、`DeliveryResult`、`CompletionPolicy`、`ReasonCode`。
- [x] `DeliveryResult` 的强 authority 分支固定为 `TransportAccepted(VerifiedTerminalRef)`、`TransportRejected(VerifiedTerminalRef)`、`TransportUncertain(VerifiedTerminalRef)`、`AlreadyTerminal(VerifiedTerminalRef)`；COMPAT/弱通道分支固定为 `BestEffortAccepted(CompatibilityEvidenceRef)`、`PartiallyAccepted(CompatibilityEvidenceRef)`、`NoChannelConfigured(ReasonCode)`、`AllChannelsFailed(CompatibilityEvidenceRef)`，另有 `Blocked(ReasonCode)`。COMPAT 分支绝不能伪装 TransportAccepted 或推进 authoritative completion；业务 Completed/NoData/Disabled 由 finalizer proposal 表达，不混入 transport 结果。
- [x] 定义现有 26 个 monitor kind 到 23 个 durable kind（含 `DailyReport` sub-kind）的逐项映射，并机械核对映射全集；其余 39 个 monitor kind 没有直接映射，每个必须按 catalog 状态选择 adapter+intent/finalizer、保持 INACTIVE，或保留 STARVED/OPT-IN 门禁，不能一概写成已接入或都要激活。
- [x] `CompletionPolicy` 至少包含 owner、advance event、NoData、Disabled、retry、uncertain/manual、already-terminal 和 finalizer kind；每个分支给出允许输入、业务转换和禁止副作用。
- [x] ReasonCode 使用稳定 namespace（`schedule.*`、`input.*`、`policy.*`、`intent.*`、`transport.*`、`finalizer.*`、`activation.*`、`shadow.*`、`operator.*`）；说明文字不得驱动状态机。
- [x] 定义 P01/N02 专用 adapter 的一致性要求、PreparedFacts 单次取数、shadow 零副作用、payload drift 进入 ResolutionRequired。
- [x] validator 先 RED 覆盖缺章节、缺类型/variant、缺 COMPAT 分支或允许其推进 authoritative completion、重复 ReasonCode、未命名空间、26→23/剩余39及14/65/52 数量断言漂移、引用不存在的 evidence/Unit、出现未决占位词。随后实现 GREEN。
- [x] 运行 RFC 测试、输入测试、现有 source/catalog 测试、两个真实 CLI draft 和 `git diff --check`；全部内容门禁通过。
- [x] 独立规格复核 Q61/Q69/Q72--Q75/Q78/Q85--Q89/Q101 及现有 Rust 类型映射；独立质量复核深模块接口、单一真相和 validator 反例。修复后提交 `docs: specify push application contracts`。

## Task 3：SQLite DDL、状态机、跨库顺序与崩溃恢复

**文件：**

- 新增：`docs/push-system/push-system-foundation.v1.sql`
- 修改：`docs/push-system/push-system-implementation-rfc.md`
- 修改：`scripts/architecture-docs/rfc_spec.rb`
- 修改：`scripts/architecture-docs/test/rfc_spec_test.rb`

**接口：** SQL 可由 `/usr/bin/sqlite3 TEMP_DB < push-system-foundation.v1.sql` 从空库执行，也可重复执行而不破坏现存行。RFC 使用 `RFC-SQL-BEGIN/END` 包住唯一一个 `sql` fence；validator 提取 fence 内原始字节（不含 fence/marker），与独立 SQL 文件二进制一致，并校验嵌入 SHA。

- [x] 先写失败测试，证明 SQL 缺表/约束、RFC 嵌入漂移、二次执行失败、非法状态插入、重复 intent payload 漂移静默覆盖、promotion journal 更新/删除会被接受时测试失败。
- [x] 定义业务库 `push_intents`：稳定 identity、Unit/业务日/occurrence/owner、payload/evidence/source-contract/template hash、业务状态、lease owner/until/generation、expected version、ReasonCode、时间字段；CAS 更新且 identity 不含 payload hash。
- [x] 定义仅追加 `push_intent_transitions`，包含前驱 hash、canonical hash、expected/result version、actor、reason 和 terminal ref identity；用 trigger 禁止 UPDATE/DELETE。
- [x] 定义版本化 `push_activation_manifests` 与仅追加 `push_promotion_journal`，包含 manifest/build/catalog/business-schema/durable-schema/template/source-contract 哈希、Unit、generation、批准者、窗口、证据和 rollback target；journal 禁止 UPDATE/DELETE。
- [x] 状态表精确定义 business intent、activation 和 authority 的合法转换、发起者、CAS 条件、持久副作用和非法路径。业务状态不复制 durable 14 态。
- [x] 跨库协议按步骤冻结：业务 intent/outbox commit → durable reserve/attempt → transport terminal → terminal ref reverify → 同一业务库事务内执行 business finalization CAS 与 append transition → commit；每步给出重启扫描和幂等键。CAS 影响零行时不得追加 transition，transition 追加失败必须回滚 intent CAS。
- [x] 故障矩阵覆盖每个 commit 前后、进程终止、DB busy、foreign lease、expired lease、Accepted 审计待定、Rejected retry、Uncertain、payload drift、expected-version 冲突、terminal ref 失效、业务 finalization 失败和回滚。
- [x] 明确不盲重发 Uncertain；`AlreadyTerminal`/人工接受仅在 exact binding 校验后推进；ResolutionRequired 阻断 Unit 晋级。
- [x] 用临时目录 SQLite 执行 DDL、PRAGMA/约束/trigger/CAS 样例和二次执行；另断言 finalization CAS+transition 同事务成功、CAS 零行无事件、事件约束失败时 intent 状态/版本不变。不得连接 `data/**`。运行 RFC/source/catalog 全套文档测试和 `git diff --check`。
- [x] 独立规格复核 Q76--Q82/Q86--Q90/Q97--Q100；独立质量复核 SQL 可执行性、append-only、防伪跨库原子性和 crash/resend 无歧义。修复后提交 `docs: define push persistence and recovery protocol`。

## Task 4：调度、readiness、shadow、activation、运维与验收合同

**文件：**

- 修改：`docs/push-system/push-system-implementation-rfc.md`
- 修改：`scripts/architecture-docs/rfc_spec.rb`
- 修改：`scripts/architecture-docs/test/rfc_spec_test.rb`

- [x] 定义 `PhaseScheduler` 的业务日 authority、occurrence identity、catch-up/restart、同 tick 合并、时窗过期和非交易日行为；盘前/竞价/盘中/盘后只做 Epic 分类。
- [x] 定义 `CoreUnready`、`ProducerUnready`、`BlockedOnInput` 的 typed snapshot、进程 exit/readiness probe、部署可观察输出和恢复事件；不得用日志代替门禁。
- [x] 定义 activation `Disabled→Shadow→Active→Draining→Disabled` 及回滚的新 generation/CAS；所有新旧 physical owner 必须读取同一 gate/fence，单个交易日只晋级一个 Unit。
- [x] 定义 shadow exact compare：共享 PreparedFacts，对比 JobDecision、SemanticProjection hash、render hash、ReasonCode 和 completion proposal；禁止 provider 二次调用、DB 写、游标推进、LLM 重算、订单或发送。
- [x] 定义 operator `inspect/reconcile/resolve-uncertain/promote/rollback` 请求/输出、typed evidence、双人/单人权限边界、审计身份、拒绝原因和 dry-run；用户/指定操作员执行生产晋级。
- [x] 定义 retention/security：非终态永不自动清理；迁移证据至少 90 天；法规/投递审计遵守 v18/v19 来源的 WORM `>5年` 要求，更严格的监管/模型/交易策略继续优先且没有统一五年上限；清理需要 terminal binding、journal 和审计。
- [x] 通用 Unit 门禁固定 unit/failure/crash/shadow/dedup/rollback 六类；当前 65-kind 基线的业务样本至少覆盖 08-31 历史补推、N02 receipt 时间、TEST_CODE G5b、NewsAI 跨 batch、09-01 254 sell、Attribution/G5b sink fail、R03、R08、NoData/Disabled/Uncertain 和双 DB 冲突/回滚。09-04 的 29 条 PaperBuy 与 Watchdog 只作为非基线设计/回放反例，不生成 Unit、不证明当前源码能力、不激活 producer。
- [x] 增加 validator 反例，保证缺状态边、缺命令、缺样本、把日志当 readiness、允许 shadow 副作用、允许删除非终态都会失败。
- [x] 运行 RFC/input/source/catalog 测试及真实 draft、SQLite DDL 执行、`git diff --check`。
- [x] 独立规格复核 Q16/Q31/Q44/Q79--Q84/Q88/Q99/Q100 和最近样本；独立质量复核 cutover 双发、回滚、权限和可观测性。修复后提交 `docs: specify push rollout and acceptance gates`。

## Task 5：W01--W21 与 52 MigrationUnit 精确 WBS

**文件：**

- 新增：`docs/push-system/push-system-wbs.v1.json`
- 新增：`scripts/architecture-docs/wbs.rb`
- 新增：`scripts/architecture-docs/render-wbs.rb`
- 新增：`scripts/architecture-docs/test/wbs_test.rb`
- 修改：`docs/push-system/push-system-implementation-rfc.md`
- 修改：`scripts/architecture-docs/rfc_spec.rb`
- 修改：`scripts/architecture-docs/test/rfc_spec_test.rb`

**接口：** `ArchitectureDocs::Wbs.validate(root)` 返回稳定错误数组；`render-wbs.rb --root ROOT --check|--write` 只更新 RFC 的 `RFC-WBS-BEGIN/END` 区间，`--check` 只读且陈旧即 exit 1。WBS JSON 是估算/依赖事实源，RFC 只保存确定性摘要。

Git 历史和当前纳管文档只保留“旧 W01--W21 合计 98--142 小时”，没有可恢复的旧名称/逐项工时。以下编号因此是依据 Q1--Q108、当前代码 seam 和 52-Unit 目录建立的**重建基线**，不是伪称找回的历史原表。WBS 每行必须保存 `lineage:"reconstructed_2026-09-06"`，并把旧总数仅作为历史对照，不强行拟合。

重建的程序工作包固定如下；W01--W20 是 Foundation/迁移支撑，W21 只交付发布编排与清理门禁本身，不把 52 Unit 的实际 cutover/tail cleanup 工时塞入 Foundation：

| ID | 工作包 |
| --- | --- |
| W01 | 身份、业务日、occurrence 与 source-contract 基础合同 |
| W02 | 应用 DeliveryResult 与现有 durable 类型适配 |
| W03 | CompletionPolicy、ReasonCode 与 RetryPolicy |
| W04 | RunContext 与 PreparedFacts 单次取数 |
| W05 | SemanticProjection、PreparedPush 与 exact bytes 绑定 |
| W06 | catalog/Unit/completion-owner 运行时注册表 |
| W07 | business intent schema 与迁移 |
| W08 | append-only transition/outbox |
| W09 | VerifiedTerminalRef 构造与重验证 |
| W10 | 通用 finalizer 与业务 CAS |
| W11 | reconciler、lease/fence 与启动恢复 |
| W12 | transport authority port 与通用 adapter |
| W13 | P01/N02 专用 conformance adapter |
| W14 | PhaseScheduler 与 occurrence catch-up |
| W15 | readiness/operational snapshot 与 deploy probe |
| W16 | activation manifest、generation CAS 与 owner fence |
| W17 | shadow harness 与 typed diff |
| W18 | operator inspect/reconcile/resolve/promote/rollback |
| W19 | 指标、SLA、保留期与安全审计 |
| W20 | fault/replay/dedup/rollback 回归 harness |
| W21 | 发布编排、N/N-1 兼容和逐 Unit/tail-cleanup 门禁工具 |

- [ ] 先写 RED 测试：不是恰好 W01--W21、lineage 缺失、不是恰好目录 52 Unit、重复/缺/额外 Unit、坏依赖/环、估算非正数或不满足 O≤M≤P、错误 PERT/汇总、缓冲重复计入、缺 phase/owner/risk/acceptance/外部等待/观察/晋级字段、RFC 摘要陈旧。
- [ ] JSON 顶层固定 `assumptions/contingency/engineering_totals/trading_totals/calendar_scenarios/critical_path`。每个 W/Unit 固定 `id/name/scope/lineage/evidence_or_catalog_refs/dependencies/risk_class/optimistic_hours/most_likely_hours/pessimistic_hours/pert_hours/engineer_count/external_wait_business_days/calendar_constraints/acceptance_gates/rationale`；Unit 另含目录 completion owner/phase/producer 快照哈希、`changes_physical_owner`、`approved_promotion_rank`、`promotion_sessions`、`observation_sessions`。
- [ ] 三点估算以 8 小时工程日计算，`PERT=(O+4M+P)/6`；一个 Codex 开发者串行实现，评审/修复计入工时。contingency 只在汇总层按明确百分比计算一次；外部等待、生产晋级和观察 session 单列，不藏入单元 PERT，也不与工程缓冲重复相加。
- [ ] 工程时间输出总 PERT 小时、8 小时等效工程日和明确百分比缓冲后的区间；交易时间输出单 physical owner、每个 Unit 所需 promotion/observation session 的下限；日历时间说明周末/休市、样本、人工批准和外部依赖，不把交易日直接当自然日。
- [ ] 依赖图无环；每个 Unit 至少依赖 W01--W03/W06--W12/W16--W20 中适用项，并按真实 owner/风险补充业务依赖。最近数据只能提高设计、回放和预修复分析优先级；生产 physical-owner 晋级顺序必须保留 Q44 已批准的 10 个 P0 Unit 顺序，未经新产品裁决不得以流量排序覆盖。
- [ ] 52 个 Unit 逐项写三点估算和专属验收证据，不用统一乘数批量填充。`MU-cli-replay-force`、startup recovery、STARVED/OPT-IN owner 必须有不同的操作/恢复门禁。
- [ ] renderer 生成 Foundation 表、四 Epic/52 Unit 汇总、首批关键路径、工程/交易/日历公式和完整 Unit 附录；重复 `--write` 字节相同。
- [ ] 运行 WBS/RFC/input/source/catalog 测试、真实 `--check`、SQL 临时执行和 `git diff --check`。
- [ ] 独立规格复核 Q41/Q44/Q62/Q74/Q93/Q102、52 Unit 一一映射和最近流量优先级；独立质量复核估算可复算、依赖无环、关键路径和晋级日历不混淆。修复后提交 `docs: add exact push migration WBS`。

## Task 6：第三批整体验证、交接与未完成边界

**文件：**

- 新增：`docs/push-system/implementation-batch-3-results-2026-09-06.md`
- 修改：`docs/push-system/README.md`
- 修改：`docs/push-system/implementation-batch-3-rfc-wbs-2026-09-06.md`

- [ ] 从干净 HEAD fresh 运行所有 `scripts/architecture-docs/test/*_test.rb`；记录 test/assertion 数和退出码，不引用旧输出。
- [ ] fresh 运行 `check-rfc-inputs`、`check-sources`、`check-catalog --draft`、`check-rfc --draft`、`render-catalog --check`、`render-wbs --check`；内容门禁全部通过。
- [ ] 在临时目录执行 SQL 两次，验证 schema、CHECK、append-only trigger、CAS、foreign key 和 journal；确认未打开 `data/**`。
- [ ] 运行全部本批 Ruby 文件 `ruby -c`、JSON parse、WBS 计数/依赖/总数、`git diff --check`。
- [ ] `check-rfc --check` 明确检查发布层并使用稳定原因码：`rfc_status_provisional`、`wbs_status_provisional`、`rfc_html_missing`、`ci_rfc_gate_missing`；对应负例进入测试。现阶段 strict 只允许这些发布层原因码；`check-catalog --check` 只允许既有 catalog/manifest 两个 PROVISIONAL。任何内容、SHA、coverage、SQL、WBS 或 freshness 错误均不得归入预期失败。
- [ ] 比较 `src/**/*.rs`、Cargo.toml、Cargo.lock 相对 `07781bf` 为零差异；比较原工作区 160 unmerged 数量和既有保护样本指纹不变。
- [ ] 最终独立 Spec 审查逐条核 Q1--Q108、硬化计划任务 2 和本计划验收；最终独立 Quality 审查接口深度、跨库恢复、SQL、WBS/周期和测试。所有 Critical/Important 必须关闭；Minor 必须修复或在结果中明确接受及成本。
- [ ] README 只把第三批标成“RFC/WBS 规格完成”；结果文档必须列出运行时未改、HTML/CI/蓝图拆分未做、52 Unit 未迁移、未部署/未真实接收。
- [ ] 提交 `docs: complete push RFC and WBS batch`。不 merge、不 push、不 deploy，不删除隔离 worktree。

## 第三批完成定义

只有同时满足下列条件，第三批才可宣告完成：

1. 八份输入逐字节冻结且 SHA 门禁可复现；65/67 冲突有明确 authority，不借冲突工作区扩大实现范围。
2. RFC 对要求的类型、variant、状态、接口、故障、恢复、调度、activation、shadow、operator、retention 和验收无未决占位。
3. SQL 可从空库和已初始化库重复执行；append-only、CAS、hash/identity、非法转换和 rollback journal 有可执行证据。
4. W01--W21 完整；WBS 与目录恰好 52 Unit 双向闭合；每项有三点估算、依赖、风险、门禁和晋级 session，汇总可复算。
5. draft 内容门禁全绿；strict 只报告第三批明确未交付的发布层阻断。
6. Rust/Cargo 和生产状态未变；独立双轴审查没有未关闭 Critical/Important。

## 本批明确不交付

- 不改 Rust runtime、数据库 schema、生产配置、消息模板或业务行为。
- 不完成蓝图 §24/§25 去拟议化、通用离线 HTML builder 或 CI 接线；它们属于下一文档批次。
- 不实现 W01--W21，不迁移 52 Unit，不做 shadow/live promotion。
- 不证明 deployment、TransportAccepted、用户已读、交易结果或收益改善。
- 不解决原工作区 160 个 merge conflict，也不吸收 PaperBuy/Watchdog 的未合并源码。
