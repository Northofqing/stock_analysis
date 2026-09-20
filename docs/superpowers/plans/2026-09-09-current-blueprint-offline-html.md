# 当前架构蓝图与第二离线HTML目标

日期：2026-09-09；更新：2026-09-10。状态：本计划完成。Task1在c1e24d0完成18节完整蓝图、65/102/52精确投影、来源与链接验收；Task2源码abeabf6完成两处MD小修、55项/1128断言及真实双HTML/浏览器验收，实际制品已在3283eaf纳管。两Task均通过独立审查；最终整体审查三项Minor由fa991e2收口，新增2项/14断言通过、唯一限定复审3/3关闭，无新增问题。前置[强制当前源码审计](2026-09-09-current-source-audit.md)最终身份采用047b4ab；九份固定来源与七份额外Git资料分域不变。完整W01–W21/52Unit目标仍未完成，证据与剩余边界见[实施记录](../../push-system/implementation-current-blueprint-2026-09-10.md)。

目标：落实原文档硬化计划的当前蓝图、两份HTML、兼容入口和统一新鲜度门禁。保留八份冻结输入和九份设计来源的原字节，以新路径记录真实当前架构；当前事实、代码已实现但未生产接线、未来RFC合同和历史材料分开。

证据输入：[当前审计增量](../../push-system/current-code-audit-delta-2026-09-09.md)、[蓝图规模与运维边界核对](../../push-system/current-blueprint-inventory-2026-09-09.md)、已批准Q56/Q65/Q70/Q95/Q105/Q107及前置机器审计。原蓝图§24/25的未来协议不复制为当前实现；未完成的W01–W21/52 Unit迁移、认证/激活、真实CI和生产验收继续保留。

## Global Constraints

- 仅在隔离树 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`、分支 `codex/push-reliability-20260905` 开发。不得修改根工作树、Rust/Cargo/config、生产monitor/数据库/凭据/provider/消息/订单、部署或远端Git。
- 不覆盖 `docs/Project_Architecture_Blueprint.md/.html`、RFC/SQL、历史catalog/manifest、WBS、八份输入及九份v18/v19来源。当前审计角色不升级为运行时权威；所有新产物保持PROVISIONAL。
- 新蓝图固定路径为 `docs/architecture/current/Project_Architecture_Blueprint.md` 与同目录 `.html`；这是文档布局内的实施选择，不新增产品或生产权限。原RFC HTML输出不改名。
- 保留Ruby2.6标准库、现有本地Mermaid原字节和模板安全边界，不引入依赖或网络安装。不执行Markdown、图源或冻结程序内容；仅专用临时浏览器profile可用于已授权的离线验收，不操作用户浏览器/生产monitor。
- 一个实施者独占当前Task代码及测试；主线负责中文交付、证据、Git及最终实际产物验收。强制current审计的共享文件必须先通过限定审查再交接。只读检查不修复或生成任何文件，生成命令仅写固定派生产物。

## Task 1: 当前架构Markdown及其证据覆盖

新建 `docs/architecture/current/Project_Architecture_Blueprint.md`。不得简单复制旧蓝图并刷新行号，亦不得用一个推送摘要替代完整项目架构蓝图。

1. 起始位置明确源码pin、当前机器catalog/manifest固定路径和原字节SHA、历史来源关系、PROVISIONAL、静态核对范围与未验证边界。使用下方“固定来源身份格式”，SHA须取前置审查修复后最终制品，不沿用初版值。Rust/Cargo pin复用已验收current审计；CI/config/README等不在push manifest内的事实另记录适用源码提交或文件证据，不能冒称由548项manifest覆盖全仓。
2. 覆盖旧蓝图的有效架构面：阅读规则/统计；系统上下文及进程/外部host边界；Data Gateway/gRPC；monitor与CLI控制面；业务能力/selection；投递、事件、数据库；AI/LLM；依赖方向；认证/配置/错误/日志；Rust实现模式；测试/CI；构建/运行；扩展约束；推送专项；设计来源吸收状态；维护与模块/target/schema目录。可以合并重复章节，但交付记录须逐项列出旧章节的保留、重写、历史化或转为RFC链接，不能静默丢失架构面。
3. 每个事实性段落、图中关键边和表格行提供当前file+symbol/行号或正式机器evidence引用；引用可定位且所属源码版本明确。默认调用、条件分支、纯库声明、cfg(test)、不可达原型、外部依赖与部署状态分别表述。未核对的调用边不得靠旧CURRENT标签或注释补齐。
4. 使用已完成594个全仓Rust文件/445884物理行、62公开模块等静态核对，和548项push manifest区分；28 binary/41 integration targets只能称静态候选，除非实际另获metadata证据。Foundation库模块不能标成monitor已启用，生产拒绝和测试绑定保留。
5. 四时段业务目录必须覆盖前置正式current材料中的全部65 kinds、102 producers、52 Units及enum外路径，以引用/表格说明触发、输入、完成权威、失败处理和现存问题。身份/owner/phase/status与已验收材料一致；代表性架构evidence不是业务迁移证书。复杂关系优先小型图/表，不逐行堆重复叙述。
6. 认证默认关闭、CLI与daemon差异、monitor固定DB身份、TOML仅启动加载、LocalBridge token、未证明的文件权限、metrics原型、CI遗留e2e引用、建议启动顺序与实际调用顺序，采用已完成核对的准确边界。不通过写文档替代修复遗留代码或认证交付。
7. v18/v19用当前来源覆盖和实际吸收证据说明；逐份覆盖固定source catalog的九份原文。git ls-files已确认另有v18.0四篇、v19.3和两个README，共16份已跟踪文件；对额外七份也读取并记录各自Git基线/文件证据和实际吸收边界，但不得将它们加入九份冻结source catalog或升级为新批准来源。撤回此前“隔离树没有这些文件”的错误前提；未读的原文只能标未核对，不得虚构覆盖。未来DDL、状态协议、排期、人力基线等链接规范RFC/WBS，不重新发明或平移为当前合同。代码推断的决策须标为推断，不新建或冒称正式批准ADR。
8. 主线静态验证所有新增相对链接、具体源码锚点/行界、当前JSON SHA/身份/数量和旧章节覆盖；独立审查核对重要调用边与状态标签。没有当前机器材料或某架构面未核对时，本Task不得完成。

完成边界：一份有完整覆盖说明、可定位证据且不覆盖旧输入的当前Markdown；尚不代表第二HTML、两目标checker或远端CI通过。

### 固定来源身份格式

当前审计最终047b4ab的字段为schema_version/status/role/baseline_commit，路径与原字节绑定已确定；已完成的两次摘要修复不改变这个接口。因此Task1与Task2共用以下封闭格式，不另创可扩展metadata系统：

- 新蓝图首行为唯一文档H1，紧接空行和一个顶层、无缩进的`architecture-source-v1`围栏；其内容为单一JSON对象，结束围栏之后才进入解释正文。不是普通叙述中的示例块，不解析任意路径或运行JSON内容。
- 顶层字段恰为`schema_version`、`status`、`role`、`baseline_commit`、`catalog`、`manifest`。前三值分别为数字1、PROVISIONAL、current-source-audit；baseline_commit为两份实际current JSON共有的源码pin。
- catalog与manifest子对象各只有`path`与`sha256`。path分别严格等于`docs/push-system/push-current-capability-catalog.v1.json`及`docs/push-system/push-current-evidence-manifest.v1.json`；sha256取各自完整原始文件字节，不对JSON重排后再算。
- Task2在强制Catalog验证成功后，比较全部固定字段、路径、两个材料的角色/状态/pin及实际原字节SHA；不能只认字段存在、只比较两份声明互相相同或只验证文件名。缺失、错位、未闭合、重复来源围栏、非JSON/非对象、字段形状不符、路径/pin/摘要不符均拒绝，且不得先写输出。
- 围栏作为可读代码块由既有MarkdownRenderer安全转义，HTML仍内嵌完整Markdown原字节。本块声明的是当前审计来源，不是运行时授权，不改变RFC metadata或旧输入。

Task1只按此格式写入实际最终材料；Task2再实现对应验证及正反例。此格式裁决本身不是第二target已实现的证据。

## Task 2: 第二target、兼容命令与实际离线验收

修改 `scripts/architecture-docs/html_builder.rb`、`build.rb`、`check.rb`、`test/build_test.rb`、`test/check_test.rb`及 `browser_smoke.mjs`；只在真实fixture需要时扩展现有support helper，不复制整套HTML/校验实现。为避免每个来源字段反例都重复全项目验证，允许把现有`test/catalog_test.rb`中的最小真实Git/current构造机械抽取为唯一`test/support/catalog_fixture.rb`供原测试与新HTML测试复用；不改其合同，不mock正式Catalog，以相关fixture/B→C/current反例定向复验，完整项目fixture及实际树验收仍保留。新增 `scripts/render-architecture-blueprint-html.rb` 薄兼容命令及新蓝图HTML，必要时重生成既有RFC HTML以匹配新的实现fingerprint。原模板/renderer/资产非必要不改。

批内承接Task1两处小修：当前新MD的v19.1行将“五交易日完整验证闭环”改为“原设计的5日收益验证闭环”（原文没有定义交易日口径）；旧A.1覆盖行的链接标签去掉src/lib.rs两侧反引号，保留原文件及#L1833目标，使既有renderer生成真实链接。只改这两处新MD内容，不修改冻结来源、不扩展通用inline parser；Task2报告记录新MD SHA，并在实际HTML中验证A.1链接存在。Task1的ab3422d2摘要仅是本Task开始前版本，不能沿用为修改后的HTML源字节证明。

1. 以封闭的 `rfc` / `blueprint` 目标表替代builder内硬编码source/output。保持 `HtmlBuilder.build(root,target,check:)`、`check(root,target)` 公共接口；所有metadata、错误target和路径检查取自同一目标定义。未知target失败，不增加任意路径或插件入口。
2. RFC前置继续冻结输入检查；blueprint生成/检查在任何输出写入前必须调用正式 `Catalog.validate(root, strict: false)` 的强制历史/current校验，并校验蓝图声明的current来源身份/原字节SHA与实际材料一致。不得解析 `.planning` 候选JSON来替代正式目录，也不得接受正文声明旧pin而current JSON已演进的陈旧蓝图。
3. 新蓝图来源声明采用Task1“固定来源身份格式”，字段仅承载角色/源pin/current路径与SHA，不创建新的业务权威或把全部Markdown当可信配置。实现其全部拒绝条件及正反例；不允许只验证字段存在而不比较实际值。
4. `build.rb rfc|blueprint [--root ROOT] [--check] --draft`以及 `build.rb --all ...`明确处理两个目标，固定顺序rfc→blueprint；拒绝单target与--all混用、重复模式或多余位置参数。保留显式draft和无draft的PROVISIONAL拒绝，不隐式发布。
5. `--all --check --draft`即使一个目标失败也检查另一个，输出各target具体错误，任一失败总exit1；全程只读。普通--all生成允许按固定顺序成功写入前一个目标、后一个失败，但须准确报告结果，不能宣称跨文件原子事务或全部成功。
6. 新兼容命令固定转发 `build.rb blueprint`并保留参数、stdout/stderr/退出码；不require执行CLI、不复制renderer、不隐式加draft。旧同名脚本实际不存在，因此不虚构其无参数行为；帮助及不带draft拒绝都做真实等价测试。
7. HTML继续内嵌完整Markdown原字节、模板/实现/固定Mermaid身份，保持中文、搜索、主题、折叠、打印、图源及安全渲染。wrapper不影响输出字节时不加入实现fingerprint；必要builder更改会使RFC HTML失效，须正式重生成而不是忽略其freshness。
8. 统一 `check.rb --draft|--check`明确覆盖两个HTML，任一缺失/陈旧仍检查独立目标及RFC/WBS/两份目录Markdown。actual draft须全部内容通过；strict只保留真实PROVISIONAL/dirty等发布阻断。CI原精确strict命令自然消费新门禁，不增加跳过条件，也不把本地声明算真实run通过。
9. TDD首条先取得未知blueprint target的真实RED终态，再做最小第二目标；随后逐项覆盖目标互不修改、all顺序/聚合、blueprint单独陈旧而RFC新鲜、current来源漂移写前拒绝、wrapper等价、缺失/无效root、source/output/父目录链接及输出hardlink。放置冻结旧蓝图sentinel，断言任何命令均不改其bytes/mtime。既有RFC行为/安全边界复用未变证据，不能以重构为由降级。
10. 最终运行受影响Ruby测试一次合批、必要语法/限定diff；主线实际生成两份HTML，重复生成不改bytes/mtime，两目标check只读；记录真实命令、metadata和内嵌源字节比对及文件SHA。不得用临时简化蓝图替代实际新文档验收。
11. 浏览器脚本新增blueprint profile，使用实际标题/表格/Mermaid数量和该文档搜索词，不沿用RFC的117/54统计；既有--endpoint/--rfc/--diagram入口仍可单独运行，不把--blueprint变成所有模式必填，也不能漏验被显式请求的蓝图。复用既有交互和网络拦截，实际验证新蓝图的图表、失败回退、全屏、搜索/主题/折叠/打印和页面HTTP(S)请求尝试为0；不重复已关闭且未变的RFC安全调查。浏览器平台日志与页面网络尝试分开，关闭仅自有验收进程。
12. 固定Task BASE..SOURCE做独立规格/质量审查，父线精确纳管新MD/HTML、必要RFC重生成及中文交付/导航；记录所有未完成内容与发布阻断。生成HTML内的官方Mermaid原始尾随空白只保留既有窄范围例外，不增加全局豁免。

完成边界：当前蓝图Markdown及其HTML、原RFC HTML、新旧兼容命令、两目标build/check、真实离线浏览器证明及实际当前draft内容通过。真实远端CI、发布批准和完整运行时迁移仍按总目标继续，不因本地完成宣称上线。

## 依赖与回滚

前置current审计 → Task1当前蓝图 → Task2双目标实现/派生产物/实际验收。来源字段已依据实际交付schema固定，具体SHA取已验收047b4ab制品（完整值见前置实施记录）；不为不存在的API写实现。路径、权限和覆盖范围已明确，无需仅因文件命名等待用户。回滚以本批代码与派生产物一致的反向提交进行，不覆盖/删除冻结输入或生产状态，不把回滚扩大为远端Git操作。
