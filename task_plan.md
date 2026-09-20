# Architecture Blueprint 重制计划

## 2026-09-06 继续：第三批实施 RFC 与 WBS

用户要求继续；按现有硬化计划和108项批准决策推进，不重新开放产品选择。目标是在隔离分支中纳管RFC所需的蓝图/最近数据输入，产出精确合同、DDL、状态转换、故障矩阵、52个MigrationUnit估算和发布门禁；仍不改运行时Rust、配置、数据库或真实推送。

- [complete] III1：第三批六任务计划已在隔离分支提交为`0bc6a2e`；独立计划审查最终Spec/Quality PASS，无未关闭Critical/Important/Minor。
- [complete] III2：八份输入共1,221,011字节逐字节纳管；输入CLI 12/238，独立Task1审查Spec/Quality PASS，提交`bb3eccd`，计划状态提交`7143065`。
- [complete] III3：RFC领域合同已中文化并固化为机器可检验规则；Task2实现`1280baf`、修复`d6f516d`、状态提交`7e47af1`，独立复审Spec/Quality PASS，71/1165全绿。
- [complete] III4：可执行SQLite DDL、跨库状态转换和崩溃/恢复矩阵已完成；实现`5713b38`、硬化`f226d0f`、状态`d763d5f`，独立复审Spec/Quality PASS，101/2161全绿。
- [complete] III5：PhaseScheduler/readiness、shadow/activation/cutover/rollback、操作员CLI和验收门禁已完成；实现`824da74`、occurrence CAS修复`507512a`、状态`246b931`，独立限定复审Spec/Quality PASS，245/3585全绿。
- [complete] III6：W01–W21与52个Unit精确WBS已完成；实现`846fa1b`、批准/精度修复`edbb509`、状态`d45dada`，独立复审Spec/Quality PASS，312/4011全绿；单开发者基线828.99h，20%缓冲后994.79h/124.35工程日。
- [complete] III7：Task6完成无占位扫描、来源/目录交叉核验、全套fresh验证及三轮CI门禁硬化；最终Spec与Quality均PASS（0/0/0），状态提交`aad7ac1`并经status-only复审PASS。clean `e35800a`五套418/5023全绿；仍不合并、push、部署。

第三批裁决：旧硬化计划任务2在一个任务中混合完整RFC、跨库协议、运行门禁和估算，无法形成独立review gate；保留全部验收要求但拆为顺序切片。代价是增加任务/提交/审查次数。正式分支缺蓝图和最近五日输入，先按原字节纳管并记录SHA；代价是第三批多一个来源治理前置，但避免RFC引用只存在于160冲突主工作区的文件。

计划复核修订：26个monitor kind映射为23个durable kind，剩余39个无直接映射；COMPAT/BestEffort使用独立`CompatibilityEvidenceRef`且不得推进权威完成；business finalization CAS与transition追加必须同一业务库事务；Q44生产晋级顺序不被最近流量建议覆盖；W01–W21旧明细无法从当前文件或Git历史恢复，因此标记为`reconstructed_2026-09-06`重建基线；文档就绪与四个运行/生产里程碑分开。

错误记录：首次按项目内`.agents/skills/subagent-driven-development`读取技能失败，该技能实际位于`/Users/zhangzhen/.codex/skills/`；命令在首个失败后未读取第二技能，已使用正确路径完整读取两者。

## 2026-09-05 继续：第二批Foundation前置目录与证据

用户“继续”授权沿用既有设计和隔离分支07781bf推进下一批，不合并/部署/修改生产库，不发送消息。依据Q29/Q59/Q64/Q91/Q93/Q104，先固化精确来源/推送目录和可机械校验的代码证据；不跳过Foundation提前晋级Paper/Watchdog。

- [complete] II1：隔离07781bf基线干净、monitor离线构建通过1.32s（84既有warnings），完整108决策/既有硬化设计及命令行测试接口已复核。
- [complete] II2：第二批计划/精确ignore规则提交64966b7，独立ledger/任务brief已建立；保留原来源字节，原目录仅入口和planning。
- [complete] II3：Task1–Task7完成；最终机器目录为65 kind/102 producer/52 Unit/195 evidence/469冻结文件，含15个普通启动恢复和10个枚举外入口；没有用占位owner或混合工作树行号填充。
- [complete] II4：来源12/110、目录32/355、draft/render、clean strict预期状态、保护指纹及独立Spec/Quality审查完成；最终技术审查基线`767a76e`。中文结果与总状态已同步，明确完整RFC/离线蓝图/CI/运行时Foundation/部署仍未完成。

本轮错误记录：最初按错误文件名查找documentation-hardening-plan-2026-09-03.md，实际为push-documentation-hardening-plan.md；隔离分支未含.agents目录，技能从原目录读取；大段批量读取发生截断，改用单文件限量读取必需指令。未发生产品代码或数据修改。

## 2026-09-05 推送可靠性实施（当前任务）

用户已说“好的 解决吧”，授权从分析进入实现；沿用已确认方案，不重新进行产品范围访谈。优先落实Foundation及漏通知、测试隔离、错误分类、完成状态相关切片，仍不自动生产晋级、发送消息或修改生产数据库。

- [complete] I1：重新检查开发基线、既有计划与隔离条件。
- [complete] I2：用户已“允许”；已从 a673043 创建 `.worktrees/push-reliability-20260905` / `codex/push-reliability-20260905`；原master及160个unmerged索引项不动。
- [complete] I3：独立开发区首批计划已写入docs/push-system；monitor构建、R08旧28测试、alert旧5测试、G5b旧13测试通过。既有dead_code warnings明确记录，不冒充全项目通过。
- [complete] I4：首批R08类型化错误及告警/G5b来源隔离已实现，源码提交至2f07ac2，分别通过独立复核。开发区样本保全偏差已披露，不冒充保全通过；新gateway_source需兼容回退制品，单owner规则不变。
- [complete] I5：首批源码90275fe、中文计划/结果和原docs入口已交付；整批定向回归、增量构建和最终修订复核通过，无剩余代码审查阻塞项。未合并/部署，完整Foundation等后续工作和全仓测试门禁仍未完成；样本事故不隐去。

预检证据：当前是普通checkout，branch=master，git-dir=git-common-dir=.git；`git diff --name-only --diff-filter=U`为160项。src/lib.rs、alert_log.rs、NotificationService与产业链mode仍有stage1/2/3。源码未扫到文本冲突标记，不据索引状态武断断言必然编译失败。现有worktree均非本次任务，部分prunable；未复用/清理他人工作区。`.worktrees`已ignored，无平台原生worktree工具。

许可已获得，继续执行。仅补齐编译所需的 `client-bundle/market.proto`，与原目录SHA-256逐字节相同；不复制 `.env`、数据库或日志。首批处理R-08错误语义及告警/G5b测试隔离，不宣称整个Foundation或全量迁移完成。

## 2026-09-05 全量重新分析（当前任务）

目标：结合全部会话约束、HTML/Markdown 蓝图、Git/冲突状态、当前推送代码与 08-31 至 09-05 本地运行证据，独立复核上一轮推论，交付 docs/push-system 下可复查的中文分析报告。既有用户授权覆盖分析文档，无需再次征求落盘许可。

- [complete] R1：冻结文档/代码/运行证据基线，恢复 108 项决策，识别来源不确定性。
- [complete] R2：全量67项四时段审计视图及关键多producer校正；冲突部分保留“原断言/待干净基线复核”，未冒充正式capability catalog。
- [complete] R3：重算最近五个交易日，复核 NewsAI/Paper/G5b/Review 根因与上一轮数字。
- [complete] R4：重审方案合同、v18/v19、迁移依赖、排期和人力；明确已批准与新增建议，未冻结未经精确WBS支持的工期。
- [complete] R5：交付 docs/push-system 中的报告、全量视图、可重跑证据和校验器；源码SHA、数量和链接验证通过。

约束：分析及文档写入；不解决用户的 160 个 Git 冲突，不发送消息，不改变产品、运行配置或数据库。冲突源只按已提交版本或明确标注的工作树证据引用，不能宣称当前构建通过。

错误记录：首次搜索包含不存在的 `.codex`，已收窄到存在目录；首次计划补丁使用不存在的 `#` 独立标题，未写入，读到实际标题后恢复。源码定位误用了不存在的 transport.rs/review_jobs.rs，已纠正为 notify.rs/review_batch.rs。首次采集查询假定 task_transition_payloads 有 created_at，SQLite 拒绝且未生成产物；读取实际 schema 后改为关联 disposition_created_at。账户查询误用securities_value，被SQLite只读拒绝，按PRAGMA改为securities_market_value。校验器最初误报Markdown标准两空格换行，改为明确允许标准hard break而保留其他行尾空白检查。初稿把intraday_monitor称为冲突源，当前Git与manifest证明为普通修改，已纠正文档。

## 真实持仓录入（2026-08-31）

### 目标

将用户截图中的 7 只真实持仓及账户快照写入项目约定的真实账户数据库，并通过数据库回读核对数量、成本、现价和汇总金额。

### 阶段

- [complete] R1. 确认真实持仓表结构、目标数据库和既有录入命令
- [complete] R2. 核验截图字段、证券代码及金额一致性
- [complete] R3. 使用项目正式写入路径保存账户快照
- [complete] R4. 回读数据库并验证 7 只持仓和账户汇总

### 约束

- 只修改真实账户快照相关数据，不触碰模拟持仓、交易流水或无关代码。
- 不覆盖截图未提供且无法可靠推导的字段。
- 写入前确认目标数据库与现有快照的替换/追加语义。

### 验证

- `PRAGMA integrity_check`：`ok`。
- 最新完整持仓快照：effective_at `2026-08-31T20:42:42+08:00`、7 项；与期望明细差异 0。
- `stock_position` open 投影：7 项；与期望明细差异 0。
- 最新账户快照：ID 8；总资产、市值、现金、可取、持仓盈亏、当日盈亏、仓位与图片 SHA 全部匹配截图。
- 写入后计数：持仓快照 23、账户快照 7、open 投影 7；相对一致性备份分别增加 1、增加 1、数量不变。

## 目标

使用已安装的 `architecture-blueprint-generator`，基于当前代码生成 Implementation-Ready 的 `Project_Architecture_Blueprint.md`，包含 C4-oriented 多层 Mermaid 图、组件与数据流、横切关注点、实现模式、部署、测试、扩展指南和代码证据。

## 阶段

- [complete] 1. 安装并完整读取 Architecture Blueprint Generator
- [complete] 2. 读取 brainstorming 与 planning-with-files 约束
- [complete] 3. 提交 bounded 文档重制设计并获得用户批准
- [complete] 4. 重新扫描当前工作树并生成 blueprint
- [complete] 5. 对账 modules/binaries/tests/gRPC/state/schema 与证据路径
- [complete] 6. 使用 verification-before-completion 验证并交付

## 设计参数

- `PROJECT_TYPE=Other (Rust)`
- `ARCHITECTURE_PATTERN=Auto-detect`
- `DIAGRAM_TYPE=C4`，用兼容性更好的 Mermaid C4-oriented flowchart/sequence/state 图表达
- `DETAIL_LEVEL=Implementation-Ready`
- `INCLUDES_CODE_EXAMPLES=true`
- `INCLUDES_IMPLEMENTATION_PATTERNS=true`
- `INCLUDES_DECISION_RECORDS=true`
- `FOCUS_ON_EXTENSIBILITY=true`

## 约束

- 所有当前态结论必须能定位到代码；推断必须显式标为推断。
- 区分 CURRENT、CONDITIONAL、COMPAT、INACTIVE、EXTERNAL。
- 不修改产品代码。
- 当前仓库 `/docs` 被 `.gitignore` 忽略，正式产物放项目根目录。

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| 安装脚本文件无执行权限 | 1 | 用 Python 运行同一官方脚本 |
| direct-download 长时间无输出 | 2 | 终止残留进程，改用 `--method git` sparse-checkout，安装成功 |
| 上一轮 root planning artifacts/ARCHITECTURE.md 在新 turn 已不存在 | 1 | 以当前工作树为准，重新建立本轮计划；不触碰现有 `.planning/.active_plan` |
| 计划状态补丁措辞不匹配 | 1 | 读取当前文件后按实际文本精确更新 |
| gRPC proto 首次查询路径写成 `grpc/market.proto` | 1 | 按 `build.rs` 的真实输入路径重新定位并核验 |
| 第二次命令尾部仍写成 `proto/market.proto` | 1 | 已确认唯一正确路径为 `client-bundle/market.proto`，后续直接使用该路径 |
| global schema fixture 首次用 tab 分隔解析 | 1 | 文件实际以 `|` 分隔；下一批按真实格式重新统计 |
| 配置/认证批次附带查询了不存在的 `src/config`、`src/auth.rs` | 1 | 按输出确认真实路径为 `src/config.rs`、`src/grpc_client/auth.rs` |
| event 批次附带查询了不存在的 `src/bus.rs` | 1 | 区分 `src/event/bus.rs`、`src/monitor/event_bus.rs` 与目录式 `src/bus/` |
| trading 批次查询了不存在的 `src/broker/` | 1 | broker 是 `src/broker.rs` 单文件模块 |
| 查询了不存在的 `src/durable_delivery/store.rs` | 1 | 实际 store/coordinator 位于 `coordinator.rs`，运行态 binding 在 monitor-local runtime |
| CI workflow glob 因不存在 `*.yaml` 被 zsh 拒绝 | 1 | 改用 `find .github/workflows -type f` 遍历真实文件 |
| 首次文档集合比对依赖 zsh command-substitution 分词 | 1 | 改用逐行 `while read` 检查每个 inventory item |

## 验证

- `cargo test --test unified_data_architecture -- --test-threads=1`：15 passed，0 failed。
- Blueprint required sections、evidence paths、18 Mermaid blocks 静态结构验证通过。
- 62/40/44/40/14/53/12/18 inventories 全量对账，missing=0。
- `git diff --check` 通过；未修改产品代码。

## 网页化扩展（2026-08-30）

### 目标

将 `Project_Architecture_Blueprint.md` 作为唯一事实源，生成可直接浏览的 `Project_Architecture_Blueprint.html`：响应式目录、全文搜索、折叠章节、18 张 Mermaid 图、源码证据与 Mermaid 失败回退。

### 已确认的公开测试 seam

- 用户打开静态 HTML 后可浏览完整架构内容。
- 页面提供目录、搜索、折叠控制和状态视觉标识。
- 18 个 Mermaid 图均有渲染容器；CDN/渲染失败时保留原始图文本。
- 页面不改变产品代码，不引入后端服务。

### 阶段

- [completed] W1. 盘点现有蓝图与本机 HTML/Markdown/Mermaid 工具
- [completed] W2. RED：运行网页公开契约验收，确认 HTML 缺失时失败
- [completed] W3. GREEN：生成完整静态架构网页
- [completed] W4. 浏览器与结构验证，修复问题
- [completed] W5. 使用 verification-before-completion 复核并交付

## 浏览器运行态验证扩展（2026-08-30）

### 目标

在不修改系统安全设置的前提下，用真实浏览器运行环境验证目录生成、搜索、折叠、主题、移动端布局以及 18 张 Mermaid 图的运行态渲染；发现问题则最小修复并重新验证。

### 阶段

- [completed] B1. 恢复 planning context，盘点可用浏览器自动化路径
- [completed] B2. 启动隔离静态服务与浏览器运行态检查
- [completed] B3. 修复发现的网页问题并回归验证
- [completed] B4. 执行最终 fresh verification 并交付运行态结论

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| Safari WebDriver session 被拒绝：Safari Settings 未启用 Allow remote automation | 1 | 不使用 `safaridriver --enable` 修改系统设置；盘点现有浏览器/cache 或采用无需该权限的替代运行时 |
| Node 验收脚本普通沙箱连接 `127.0.0.1:9222` 返回 EPERM | 1 | 按本机端口权限边界使用已批准的本地连接重跑 |
| Chrome 真实渲染未达到 18/18 Mermaid SVG | 1 | 根因定位为 HTML 中两处 whitespace regex 落盘成 `/s+/g`；以现成浏览器验收为 RED，仅修复这两处 |
| 移动菜单点击后立即检查 sidebar left 未到 0 | 1 | 先采集 body/ARIA/transform/rect 与 transition 时序，区分页面缺陷和测试竞态 |
| Chrome console 出现两条无 URL 的 404 resource log | 1 | HTTP log 定位为 `/favicon.ico`；以内嵌 data-URI favicon 消除请求，不放宽错误断言 |
| favicon 修复后 CDP 仍收到一条 404，但 fresh HTTP log 无对应请求 | 1 | `Log.enable` 回放旧 entry；reload 前清理 Log/console 和本地 buffer，使断言只覆盖当前 run |
| 查询可选历史证据时使用了不存在的 `data/private_evidence/2026-08-24/*.json` glob，zsh 在执行前报 `no matches found` | 1 | 后续改为读取已确认存在的 2026-08-19 显式文件路径，不再使用未解析 glob |
| 首次用 sqlite3 `.backup` 后，预期备份文件无法以 `-readonly` 方式打开 | 1 | 文件实际已完整产生；其 WAL 模式在 `-readonly` 下需要 sidecar。改用 SQLite `immutable=1` 只读 URI 验证，`integrity_check=ok` 且写入前计数 22/6/7 正确 |
| 沙箱内用 `ps -ax -o ...` 诊断长时间导入进程时报 `operation not permitted` | 1 | 不提升权限、不干扰服务；继续轮询原 PTY，随后导入器正常返回成功回执 |
| 首张 diagram 截图落在图前的表格区域 | 1 | 运行态 DOM 已确认 SVG 存在；改用元素 page-coordinate clip，而非依赖 viewport scroll framing |
| 最终静态 verifier 的 `/private/tmp` 文件已被系统清理 | 1 | 不依赖旧临时文件；以 apply_patch 重建最小自包含 verifier 后 fresh run |

## 最近改动与架构网页漂移审计（2026-09-01）

### 目标

以 2026-08-30 蓝图和当前代码为依据，整理最近提交及工作树改动，判断 `Project_Architecture_Blueprint.md/.html` 是否存在事实、清单、调用链或运行时拓扑漂移；本轮先报告，不擅自改写架构产物。

### 阶段

- [completed] R1. 恢复 session，确定最近改动范围与基线
- [completed] R2. 按文件/模块梳理改动及架构影响
- [completed] R3. 对照 Markdown/HTML 定位应调整与无需调整项
- [completed] R4. 验证证据并交付调整建议

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| inventory Ruby 使用了旧运行时不支持的 `Array#filter_map` | 1 | 改用 `each` + 显式收集，保持同一统计口径重跑 |
# 全部推送项业务逻辑逐行审计（2026-09-01）

## 目标

以当前工作树为唯一事实源，识别全部实际可达的推送项及其配置、触发、筛选、文案、发送、重试、审计和统计链路；对每个业务结论提供精确到文件与行号的代码证据，并明确区分已启用、条件启用、兼容保留和不可达逻辑。

## 阶段

- [completed] P1. 建立推送相关文件、入口、配置和调用点的完整清单
- [completed] P2. 沿运行时调用图识别全部推送项与共同基础设施
- [completed] P3. 逐推送项、逐代码块审计业务规则和边界条件
- [completed] P4. 反向核对测试、数据库 schema、模板和配置，查漏与验证可达性
- [completed] P5. 生成带行号证据的中文分析报告并做最终覆盖率核验

## 证据标准

- 每项结论至少引用一个当前存在的 `path:line`；跨层结论同时引用入口与落点。
- “逐行”按可执行语义逐行/连续代码块解释；纯语法闭合、导入和显然的派生样板只说明其结构作用，不虚构业务含义。
- 对测试证据与生产代码证据分栏；测试只能证明契约意图，不能替代运行时可达性。
- 未发现调用方、仅 feature/config 下启用、或仅历史兼容的逻辑必须明确标记。
- 不修改产品代码、配置或数据库。

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| 首次读取 skill 的命令因前置 `rg AGENTS.md` 无匹配而被 `&&` 短路 | 1 | 将 skill、AGENTS 搜索和 git 状态改为独立并行只读命令；skill 已完整读取 |
| 首次集合差分脚本使用旧 Ruby 不支持的 `Array#filter_map` | 1 | 改用 `map + compact`，已得到 65/54/11 精确集合 |
| `cargo test --bin monitor` 默认并行运行出现 6 个 durable runtime 隔离失败 | 1 | 精确单测 1/1 通过，BR-194 终态回放组单线程 14/14 通过；保留“全量并行未绿”结论，不把推断写成已修复 |
| 阶段集合核验脚本使用旧 Ruby 不支持的 `Array#tally` | 1 | 改为 `Hash.new(0)` 显式计数；复跑成功，65 项无 missing/extra/duplicate |
| 仓库无 HTML 生成脚本，本机无 pandoc/cmark/Markdown renderer，Ruby kramdown/commonmarker 与 Python markdown 均未安装 | 1 | 不下载依赖；保留现有 HTML 壳，使用 apply_patch 同步 article/CSS/统计，再机械更新嵌入 Markdown 与 SHA |
| 首次机械更新 HTML 嵌入源时 Ruby `File.binread` 的 ASCII-8BIT 与含中文 UTF-8 regexp 不兼容 | 1 | 未写回文件；下一次将 HTML 显式标记为 UTF-8 后再替换，保持同一锚点与 SHA 算法 |
| 首次综合校验时 Markdown 仍以 ASCII-8BIT 参与含中文章节 regexp | 1 | 校验无写入；保留原始 bytes 做 hash/base64 比较，另复制并显式标记 UTF-8 做标题扫描后复跑 |
| 综合校验最初使用 `\bid=` 提取 HTML ID，误将 `data-section-id` 计作真实 `id`，产生 34 个重复 ID 的假阳性 | 1 | 改为只匹配空白后出现的 `id=`，并增加 section/control/body 集合一致性校验后复跑 |
| Mermaid 源一致性首次按 Ruby 字符串编码比较，将 Base64 解码的 `ASCII-8BIT` 与 Markdown 的 `UTF-8` 中文行判为 6 张图不一致 | 1 | 核对长度与差异字节后确认内容相同；改用 `.b` 原始字节比较，不改写图源 |
| 系统 `/usr/bin/tidy` 按旧 HTML/字符集规则检查时，把 UTF-8 中文及 HTML5 的 `aside/nav/svg` 报为非法 | 1 | 不以旧版 tidy 作验收依据；改用 UTF-8 校验、标准库 HTML5 宽容解析、显式标签栈与 DOM 契约检查 |
| 尝试复用系统 Ruby RDoc Markdown renderer：首次漏先 require `rdoc`，修正后确认其不支持本节 GFM table | 2 | 未写文件；改用仅覆盖第 24 节现有语法的受限转换器，并对表宽、图数、替换锚点做 fail-fast 校验 |
| 第 24 节受限转换器首次用 `\A## 24` 锚定整份 Markdown，无法匹配非文件首部章节 | 1 | 未写 HTML；改为行首 `^## 24` 后成功生成 19 个小节、14 张表 |
| 首次更新新 SHA/内嵌 Markdown 时误把 Ruby 2.6 `String#gsub` 当成返回替换计数 | 1 | 在写回前 abort；改为先 `scan` 验证四个锚点均唯一，再 `gsub!/sub!` 写回 |

## 2026-09-02 按交易阶段重排

- [completed] S1. 以 `MarketSession` 的真实边界核对盘前、竞价、盘中、盘后
- [completed] S2. 为 65 个 PushKind 指定主归属时段并标注跨时段入口
- [completed] S3. 补入不经过 PushKind 的产业链报告与分析流水线通知
- [completed] S4. 单列无 producer、默认关闭和结构不可达项，保证 65 项不漏算

## 2026-09-02 蓝图文档落盘

- [completed] D1. 核对蓝图源文件、生成页结构与现有章节编号
- [completed] D2. 在 Markdown 中新增推送系统专项架构与演进章节
- [completed] D3. 机械同步 HTML 内容、目录统计、嵌入源与 SHA-256
- [completed] D4. 将 65 个 PushKind 与非 PushKind 路径按盘前、集合竞价、盘中、盘后逐项补入蓝图并附代码证据
- [completed] D5. 重新同步并校验 Markdown/HTML 标题、图表、链接、源 hash、结构与 git 状态

## 2026-09-02 推送改动方案与周期落盘

### 目标

以 Codex 单独开发为人力基线，把推送专项从“演进方向”细化为可执行路线图，明确任务包、依赖、工期、里程碑、验收门禁、回退边界与风险缓冲，并同步到 `docs/` 下的 Markdown/HTML 两份蓝图产物。

### 阶段

- [completed] T1. 定位第 24.18 节的方案与周期缺口，确认既有 65-kind 审计不改口径
- [completed] T2. 补入单开发者基线、C0-C6 改动包、2-8 小时 WBS、依赖与三点估算
- [completed] T3. 重建 HTML 第 24 节并同步嵌入 Markdown 与 SHA-256
- [completed] T4. 对章节、表格、PushKind、源码证据、HTML 结构和脚本执行 fresh verification
- [completed] T5. 将两份蓝图从仓库根目录迁入 `docs/`，并登记到 `docs/README.md` 当前执行入口

## 2026-09-02 Grill 决策树回写蓝图

### 目标

按用户确认的 Q1--Q55 决策重写蓝图第 24.18 节：废止横向 C0--C6 实施顺序和 10--15 日错误基线，改为 Foundation + 纵向 MigrationUnit + physical promotion/cleanup；补全 receipt 强度、Uncertain 分级、shadow、跨库 finalizer、activation manifest、人工裁定、生产灰度、运行门禁、真实工期和退出标准。

### 阶段

- [completed] G1. 完成十轮 Grill 并取得用户总确认
- [completed] G2. 重写 Markdown 第 24.18 节及关联术语/总索引说明
- [completed] G3. 重建 HTML 第 24 节并同步内嵌 Markdown 与 SHA-256
- [completed] G4. 执行章节、PushKind、证据、链接、HTML、JS 与禁用旧基线的 fresh verification
- [completed] G5. 审计 `docs/v18.x`、`docs/v19.x` 全部设计文档，新增蓝图 §25 覆盖/冲突/落位矩阵并同步版本 README

## 2026-09-03 推送迁移方案与 v18/v19 落位审查

### 目标

只读审查 `docs/Project_Architecture_Blueprint.md` 第 24--25 节：验证方案在接口边界、状态机、跨库一致性、上线门禁、可复现文档治理、工期与 v18/v19 依赖顺序上的自洽性，并为每条问题回到当前代码或文档给出精确行号证据。

### 阶段

- [completed] V1. 固定当前工作树审查基线并提取第 24--25 节关键契约
- [completed] V2. 对照 durable delivery、调度、通知和配置源码验证状态机与运行门禁
- [completed] V3. 对照 v18/v19 原始设计验证覆盖、冲突与实施顺序
- [completed] V4. 汇总按严重度排序的发现、优点与整改优先级

### 约束

- 本轮不修改蓝图、产品代码或配置；仅维护审查过程文件。
- 工作树存在用户并行改动，审查结论以 `git status` 与文件内容采样时点为准。
- 发现优先于摘要；每条发现必须包含可定位证据、影响与建议。

### 审查结论

- 结论为“方向通过、暂不具备直接实施条件”：纵向 MigrationUnit、authority 分层、禁止 blind resend、单 physical owner、v18/v19 状态分层均应保留。
- 开工前阻断项：跨库 intent 完整状态机与 crash matrix；不可压扁的 terminal evidence contract；manifest 对 executable/catalog/schema 的绑定与 promotion journal；Foundation-capable rollback 边界；每 Unit 测试门禁；版本控制与可复现生成/证据锚点。
- 文档治理问题：蓝图仍 untracked，部分 v18/v19 源设计 ignored/untracked，W01--W21 基础估算已从正式蓝图消失，当前 push template 并行改动已使若干行号证据漂移。

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| HTML 内嵌 Markdown 校验首次查找了不存在的 `blueprint-markdown-source` ID | 1 | 读取 HTML 后按真实 `blueprint-data` ID 重跑；SHA、body metadata 与内嵌 Markdown byte-exact |

## 2026-09-03 当前架构蓝图与网页同步更新

### 目标

以当前 HEAD 与工作树源码为唯一事实源，修正 `docs/Project_Architecture_Blueprint.md/.html` 中 2026-08-30 provider 内置架构残留，同时保留已经加入的推送专项 §24、v18/v19 覆盖 §25 与用户并行文档改动。

### 已批准设计

- 本次属于 bounded 文档改动；上一轮已向用户提交 P0/P1 调整矩阵，用户以“修改吧”批准。
- Markdown 是唯一事实源；HTML 必须由更新后的 Markdown 同步生成并保持 source hash byte-exact。
- 只修改架构蓝图及其必要的可复现生成/校验辅助，不修改产品代码、配置或用户并行改动。

### 阶段

- [completed] U1. 固定 2026-09-03 当前基线，核对新增提交、dirty worktree 与蓝图现状
- [completed] U2. 重扫 modules/targets/tests/operations/schema/task tree/热点及最新 BR-250/BR-255 影响
- [completed] U3. 更新 Markdown 当前态章节、图、清单、证据与 ADR，保留 PROPOSED/HISTORICAL 章节
- [completed] U4. 同步生成 HTML，刷新日期、目录、Mermaid、内嵌 Markdown 与 SHA
- [completed] U5. 执行架构测试、静态对账和真实浏览器验收

### 约束

- 当前工作树含用户并行修改；不覆盖 `.gitignore`、selection activation、push template 与版本 README 改动。
- `docs/Project_Architecture_Blueprint.*` 本身为未跟踪文件，但属于本次明确授权范围。
- CURRENT/EXTERNAL/PROPOSED/HISTORICAL/INACTIVE 必须明确区分；源码遗留注释不能覆盖真实构建和调用图。

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| §16--§23 合并补丁因测试图 `coverage` 精确上下文不匹配而验证失败 | 1 | 补丁整体未落盘；改为读取当前小节并按章节拆分应用，避免一个锚点阻断整批 |

## 2026-09-03 本次文档改动目标符合性复审

### 目标

以 `HEAD a673043` 与当前工作树为固定比较边界，按 Standards / Spec 双轴重新审查文档改动是否满足用户本轮目标：完整推送业务逻辑及四阶段分类、问题与优化、单人开发方案/周期、Q1--Q55 决策、v18/v19 设计覆盖，以及当前架构事实和 Markdown/HTML 可复现同步。

### 阶段

- [completed] R1. 固定 WIP diff、用户规格和仓库文档标准
- [completed] R2. 并行执行 Standards 与 Spec 独立审查
- [completed] R3. 主流程逐项复核发现并运行文档生成/结构/证据门禁
- [completed] R4. 给出符合、部分符合、不符合的最终裁决

### 审查范围

- tracked diff：`.gitignore`、`docs/README.md`、`docs/v18.x/README.md`、`docs/v19.x/README.md`、`docs/v19.x/v19.3-push-workflow.md`。
- untracked deliverables：`docs/Project_Architecture_Blueprint.md/.html`、`scripts/render-architecture-blueprint-html.rb`。
- `config/selection/**` 与 `src/bin/monitor/push_templates.rs` 是并行产品改动，只作为当前事实证据，不评价其实现质量，也不归入本次文档变更。

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| 直接执行 HTML renderer 返回 permission denied | 1 | 文件当前无 executable bit；改用文档声明的 Ruby 解释器方式复验，并检查维护说明是否准确 |
| 首次追加复审 findings 使用了只存在于 progress 的章节锚点 | 1 | 读取 findings 实际尾部后，改用当前最后一行作为精确补丁锚点 |
| catalog 复核脚本使用当前 Ruby 不支持的 `filter_map` / `tally` | 2 | 改用兼容的 `select`、`map.compact` 与 `Hash.new(0)` 计数后复验为 5/7/22/31、总计 65 且无重复 |

### 最终裁决

- 总体为**部分符合，不可按 Implementation-Ready 验收**：四时段 65-kind 清单、问题/优化、v18/v19 分层和当前架构同步主体均已落盘；但逐行证据已被并行源码修改打漂、实施合同仍有关键未定义项、工期基础 WBS 不在正式文档、Q1--Q55 无逐题追踪矩阵。
- Standards 阻断：修改后的 v19 README 仍保留已退休规则的 active pointer；§25 依赖的 9 份 v18/v19 设计稿仍 ignored/untracked；HTML 依赖未锁 CDN，renderer 以既有 HTML 反作模板，不能从独立源 clean rebuild。
- Spec 阻断：§25.8 把 Candidate/BR-232 证据指向 PaperTrade/R-12 等无关段落；`CompletionPolicy`、Prepared 类型关系、cross-DB intent 状态转移及 activation artifact/schema binding 仍不足以直接编码。
- Fresh gates：renderer `--check`、Ruby syntax、`git diff --check` 通过；65-kind 分类为 5/7/22/31，状态 37 ACTIVE/24 INACTIVE/2 STARVED/2 OPT-IN；三项架构证据测试为 15/15、8/8、1/1。

## 2026-09-03 推送文档证据链与 Implementation RFC 重构

### 目标

按用户确认的 Q56--Q108 决策，将“当前代码事实审计”和“未来推送实施设计”拆成独立事实源；补齐稳定证据 manifest、65-kind machine catalog、Q1--Q55 追踪、v18/v19 source catalog、W01--W21 与原子 MigrationUnit 估算、完整类型/DDL/状态机/crash matrix，并建立可离线从零重建的双 HTML 与自动门禁。

### 已批准约束

- `docs/Project_Architecture_Blueprint.*` 只保留当前架构、65-kind 现状审计、问题与 RFC 链接；当前 dirty baseline 标记 `PROVISIONAL`。
- 新 RFC 放在 `docs/push-system/`；四时段是 Epic，原子 Unit 按 physical producer + occurrence family + completion owner 划分。
- 本轮可修改文档、生成器、校验器与 CI，不修改或提交产品代码、数据库、运行配置和用户并行改动。
- v18/v19 九份来源原文保持不变，只通过 `.gitignore` 放行并由 source catalog 记录 SHA/裁决。
- 严格发布门禁要求干净 commit；当前工作树只允许 `--draft` 验证通过，不能冒充正式发布。

### 阶段

- [completed] H1. 固化已批准 Grill 决策、文件边界与实施计划
- [pending] H2. 纳管 v18/v19 来源并修正文档状态、退役规则指针和蓝图/RFC 边界
- [pending] H3. 完成 RFC 的类型、DDL、状态机、failure/crash、activation、shadow、promotion 与 rollback 合同
- [pending] H4. 建立 65-kind catalog、稳定 evidence manifest、source catalog、Q1--Q55 和 W01--W21/Unit 估算
- [pending] H5. 将 HTML 生成链改为独立模板和离线资产，分别从零生成蓝图/RFC HTML
- [pending] H6. 增加统一 checker 与 CI 门禁，执行 draft/strict 反向验收
- [pending] H7. 运行静态、HTML、catalog、证据和现有架构回归验证，记录剩余的 clean-baseline 发布阻断

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| planning session catchup 调用未返回可解释的 exit code/output | 1 | 已另外执行 `git status`、`git diff --stat`、最近提交和 planning 文件回读完成等价恢复；不重复同一调用 |
| spec placeholder 扫描命中实施计划中用于未来 RFC 的扫描命令文本 | 1 | 判定为命令字面量而非未决占位；逐文件复核 decision record 无占位，后续 checker 将排除 fenced code/命令示例 |
| 中文化大补丁的 `apply_patch` hunk header 格式错误，补丁未执行 | 1 | 拆分为较小的标准 `Update File` 补丁，逐段翻译并在完成后复核 |
| 决策行完整性检查误用了当前旧版 Ruby 不支持的 `filter_map` | 1 | 改用 `map` + `compact` 的兼容写法重新校验 |
| untracked 决策记录的 whitespace 检查发现头部 3 处 Markdown 行尾双空格 | 1 | 去除行尾双空格后重新执行检查 |
| 格式检查误将历史 `progress.md` 全文纳入 240 字符限制，命中既有长行 | 1 | 不改写历史记录；将该文风规则收窄到本次中文化的两份目标文档 |

## 2026-09-03 真实账户持仓快照导入

### 目标

将用户提供的东方财富普通账户截图按项目既有正式 importer 写入业务数据库，并以截图哈希、导入前备份、账户恒等式和导入后 SQL 回读形成证据闭环。

### 阶段

- [completed] D1. 固化截图字段并核对 6 只持仓市值合计
- [completed] D2. 定位既有 importer、证券代码、数据库 schema 与当日重复写入状态
- [completed] D3. 校验本次导入输入与历史备份状态，并补做已写入状态的一致性备份
- [completed] D4. 确认当日持仓与账户汇总已由正式入口落库，避免重复追加
- [completed] D5. 回读数据库、复核图片哈希和账户恒等式，记录结果

### 验证结果

- 生产库 `PRAGMA integrity_check`：`ok`。
- 最新持仓快照：ID 25，effective_at `2026-09-03T19:14:00+08:00`，6 项；期望/实际双向差异 0，且同一时刻只有 1 条快照。
- 账户汇总：ID 27，同一时刻只有 1 条；总资产、市值、可用、仓位和当日盈亏与截图一致。
- open 投影中唯一未被最新快照确认的代码为德展健康 `000813`；按 BR-215 保留，不推断平仓。
- 2026-09-03 real-account 行为 0；总账存在截图未解释的 `16260.23`，未伪造现金字段绕过 BR-103。
- 导入后状态备份完整性为 `ok`，SHA-256 为 `f19619ab1dc9ef3a6822a1c2d021df8bf8f20399e90d34521af39d347c6f5602`。

## 2026-09-03 最近四日生产推送复盘与整体方案校正

### 目标

以 `2026-08-31` 至 `2026-09-03` 的真实生产推送、durable 决策、业务完成状态、错误日志和账户输入为证据，重新检验推送系统 RFC 的优先级、模块 seam、迁移 Unit 与验收门禁；本轮只输出方案建议，不修改设计文档或运行代码。

### 阶段

- [completed] E1. 盘点真实推送证据源、表结构、日志范围和生产进程
- [completed] E2. 按盘前、集合竞价、盘中、盘后统计发送、接受、抑制、失败、Uncertain 和重复
- [completed] E3. 对账业务完成状态、游标、账户输入、durable receipt 与实际用户可见结果
- [completed] E4. 用生产事实反证现有方案，形成保留项、删减项、新增项及 2--3 个可选架构路径
- [completed] E5. 给出推荐方案、实施顺序和需要用户确认的设计变更

### 约束

- 不触发真实推送，不调用 sink，不修改数据库、配置、产品代码或当前设计文档。
- 只输出统计、哈希、状态和脱敏错误类别，不读取或泄露 token、webhook、账号等敏感值。
- “实际发送”必须由 typed receipt/durable terminal 或可验证物理通道记录支持；日志中的“准备发送/调用成功”不能单独算成功。
- 当前用户请求已授权只读生产事实复盘；任何文档或实现修改必须等待本轮建议获批。

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| 首次并行读取三个技能说明时生成了非法 JavaScript 参数，调用在执行前失败 | 1 | 改为最小 `Promise.all` 只读调用并完整读完三个技能及 deepening 参考 |
| 首次递归扫描 `data/` 把大量 `data/test/**` 测试产物纳入，输出约 900 KB 并被截断 | 1 | 后续只查询明确的生产文件、最近四个日期和生产数据库，不再扫描 test namespace |
| dispatcher 聚合的 `jq` 表达式未给 `input_filename` 子表达式加括号，导致后续 `.kind` 在文件名字符串上求值并产生大量错误 | 1 | 改为括号隔离文件名表达式，并先用单行样本验证字段后再聚合 |
| 首次按 durable `accepted_at` 字符串小时分类，未先确认其 UTC/offset 格式，导致盘后 review 被暂归盘中 | 1 | 先抽样时间格式，再统一转换为 Asia/Shanghai 后重算；废弃首次 durable 时段结果 |
| 固化时区结果时错误假定 `findings.md` 存在与 progress 相同的章节标题，首个补丁未命中且未产生写入 | 1 | 用 `rg`/行号定位现有 `2026-08-31 至 2026-09-03 真实推送复盘` 段后按稳定上下文重新补丁 |
| 第二次固化补丁在多文件 hunk 之间多写了孤立 `@@`，补丁语法校验失败且未产生写入 | 1 | 拆成三个标准单文件补丁执行，避免跨文件 hunk 格式歧义 |
| 查询 G5b analytics 明细时误用不存在的 `push_events` 表名 | 1 | 通过 `.tables` 与 `sqlite_master` 重新确认生产表名为 `push_analytics`，后续只按真实 schema 查询 |
| 查询 `selection_event_completions` 时误用不存在的 `created_at` 列，导致同一 SQLite 命令后半段未执行 | 1 | 保留此前已成功返回的 `pushed_stocks` 结果；下一步先查该表 schema，再按真实业务日期列查询并单独回读 real-account |
| 首次回读 `real_account_snapshot` 时误用不存在的 `effective_at` 列，导致该条 SELECT 未执行 | 1 | 由 `PRAGMA table_info` 确认日期/时间列为 `snapshot_date` 与 `source_captured_at` 后重新查询 |

### 截图事实

- 账户日期按用户“今天”解释为 Asia/Shanghai 的 `2026-09-03`；截图显示时间为 19:14。
- 总资产 `70,795.65`，证券市值 `45,921.00`，仓位 `64.9%`，当日盈亏 `+160.99`，持仓盈亏 `-19,054.79`，可用 `8,614.42`，可取 `7,226.66`。
- 可见且覆盖全部证券市值的 6 只持仓：利欧股份 `9,360.00`、合肥城建 `4,088.00`、达实智能 `13,560.00`、华电辽能 `13,570.00`、三安光电 `1,389.00`、建业股份 `3,954.00`。

### Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| 将大库 `PRAGMA integrity_check` 与状态查询合在一次 30 秒调用中，未在窗口内返回可见结果 | 1 | 改为先执行快速只读状态查询，再单独启动并跟踪完整 integrity check |
