# 盘后产业链：固定准备、持久恢复与真实定时器接管

日期：2026-09-11。初始源码42ce098，隔离分支codex/push-reliability-20260905。状态：执行中，尚未完成。

最新用户范围调整：[单用户本地模式](../../push-system/single-user-local-scope-2026-09-11.md)优先适用。本次不再以复杂可信身份、外部发行方、多角色审批或认证broker为前置；旧条款中的此类要求已排除本次范围，而非标为实现完成。同库事务、任务/效果身份、任务锁、防重、数据校验和实际切换授权不变。当前旧ProductionRefused是代码现状，新本地入口待接线，不能直接改成无条件放行。

## 目标与规范

整体目标仍为W15–W21与全部52个Unit交付。本计划落实MU-chain-post-close的真实业务流程，不以新helper、内存日期位、局部兼容观察或测试数量替代完整接管。

依据：[RFC](../../push-system/push-system-implementation-rfc.md)的RunContext/PreparedFacts、outbox原字节恢复、完成策略、共同fence与逐Unit六门禁；[当前蓝图](../../architecture/current/Project_Architecture_Blueprint.md)；[盘后恢复设计](../../push-system/chain-post-close-recovery-design-2026-09-11.md)。通知前置已在de38876完成；弱成功不等于强回执。

现状：app/modes.rs的run_chain_analysis_mode将采集、pipeline、报告落盘和通知混在一起，最终Ok被main.rs的CHAIN_POST_LAST当成通知完成。pipeline内部又有概念缓存、chain_daily写入、持仓读取、补充搜索、多次模型调用与首次报告组装。恢复不得重新运行整个pipeline来补齐这些步骤。

## Global Constraints

- 只改现有隔离工作树；不读取真实.env、生产DB/凭据，不运行或观察生产monitor，不调用真实provider/LLM/sink/PAM/交易，不部署、远端Git或CI。
- 一名Rust实施者，主控独占Cargo/Git/公开文档与ledger。源码冻结后才运行单Cargo队列；子代理不运行Cargo、Git写入或再派子代理。
- 测试使用显式配置、合成TEST_CODE内容、受控时钟、自有临时库/文件；HTTP如有需要只能使用有界随机loopback且由主控申请沙箱权限。不得借mock数据或测试构造器签发Production身份。
- 不改冻结RFC/DDL/目录/蓝图、旧外部CLI/配置/模板、十渠道顺序和任一弱成功语义；生产注册/迁移/owner切换分别批准。新存储代码须同既有业务库受控扩展，不能第三套旁路DB或偷塞reason/actor字段。
- Accepted/Unknown仅本地弱观察，不构造强terminal、不自动重发Unknown、不借R03/Magiclaw回执、不推进强cursor。首次报告原字节及未决事实保留，恢复不重新provider/LLM/render。
- 不处理无关旧warnings/全仓格式；只用已审核范围取得验证证据。各Task单独验收，只有整体目标终局才做整分支收尾。

## 顺序、交付与完成边界

1. Task 1实际pipeline提供固定准备结果，旧入口消费同一次流程；把可恢复内容与原有外部效果明确分开。
2. Task 2在同业务库接准备检查点、原始artifact、保存进度和逐目标尝试journal；将检查点接入Task 1的实际效果调用处，不是只测store CRUD。
3. Task 3真实app/timer/启动入口消费持久流程，完成资格不再依赖函数Ok；交易日/当下窗口/独立业务日identity可验证。认证未就绪时不得自行切换生产路径或关闭旧推送。
4. Task 4认证注册、强渠道与盘后独立cursor的原子最终化；六门禁、旧owner退出/回滚及实际发布材料。未取得强权威和部署证据就不称完整Unit完成。

实际必达渠道、受保护根/生产身份与具体切换批准尚未有新输入。独立代码工程继续；不能替用户选择所有渠道必达或把Magiclaw替代全部渠道。

## Task 1: 将固定准备结果接入实际产业链pipeline

### 约束与所有权

工作目录：`/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`。主控负责Cargo、Git、公开文档；实施者不运行Cargo/Git写入/生产命令、不派子代理。测试不读环境或生产数据，只能显式合成配置/输入、临时自有数据库或经主控批准的loopback。只读本任务brief，不读其他计划私有目录。

可编辑：`src/pipeline/chain_analysis/mod.rs`、`src/pipeline/chain_analysis/fetchers.rs`；新增同目录`preparation.rs`、`preparation_tests.rs`；`src/gate_d_chain_analysis_regression.rs`仅迁移其旧renderer失败协议测试至同一新公开interface，不删除文件或降低真实loopback Gemini协议覆盖。额外路径须先解释并由主控更新边界。不改app/modes.rs、timer、通知、Foundation或生产schema。本Task结果是后续持久准备的真实输入，不宣称已实现重启恢复。

### 行为合同

1. 现有公开run_chain_analysis仍返回Result<String>，但必须委托同一次新的准备流程再投影原报告。新的生产可用interface返回只读结构化准备结果：固定business_date、调用提供的limit_ups/macro输入、实际采用的准备事实/模型结果和首次报告原字节。不得平行复制一套业务循环，也不能只在test下运行新流程。
2. 将实际外部效果放在可替换的seam：现有生产adapter使用原provider、数据库和模型；受控adapter只替换外部I/O/时钟，不mock内部聚类、tier选择、提示词构造或build_report。默认adapter不在进入空涨停池分支前读环境、访问DB或创建真实模型。
3. 准备结果保留pipeline实际消费的涨停股、概念映射、聚类/孤立股、持仓诊断、补涨候选及来源状态、龙虎榜背景、宏观/逐簇/盘后催化背景和首次报告。不能只保留hash或由渲染文本反推这些事实。没有源时间/批次证据的旧来源显式标识未提供，不写当前时间冒充来源时间，不把普通空Vec认证成VerifiedEmpty。
4. 对本流程实际模型调用保留发送到analyzer的prompt/system/mode以及其返回文本或失败/未调用状态，包括用于生成检索词的调用。这些是本地调用材料，不冒充底层真实provider选择/模型版本或远端原始wire；无法取得的身份明确缺失。不要为了凑字段扩改整个GeminiAnalyzer。
5. 首次报告、模型响应及文本保留原字节/空白；序列化artifact须明确版本，确定性编码自身拥有的映射，不改原报告/选集的排序。拒绝未知版本、截断/不合法输入；解码不得触发外部调用。Debug/诊断只展示安全元数据，不输出持仓/报告/提示词/模型内容或凭据。字段私有、getter只读；序列化artifact本身包含业务内容，只能由后续受保护存储消费，不当日志。
6. 保持现有成功/失败/降级政策及副作用次序：核心概念/DB失败仍阻断，补涨不可用保留真实原因而不清空核心分析；新闻/模型可选失败仍按原路径降级。龙虎榜当前按请求时Local自然日取数，不擅改成pipeline业务日；应保存实际请求日期与本地观察时间，不冒充provider时间。chain_daily及概念缓存仍是原适配器中的真实副作用，必须在说明中标明；本Task不把含写库的准备假称为纯函数或已持久checkpoint。能在产生观察处保留的错误/缺失不得再次压成空String；被上游接口已丢失的信息明确标识Unknown，不伪造恢复。
7. 下一Task将把持久begin/result围绕这些实际外部效果放置。保留可明确定位的stage职责（概念、聚类业务写入/生命周期、候选、持仓、龙虎榜、宏观、搜索/模型、首次报告）；不新增通用工作流引擎、任意JSON事件总线、第三DB、全局可变测试开关或自动重试器。

2026-09-11编码细化：artifact仅接收本编码器产生的版本化、确定性紧凑原字节；解码后重编码须逐字节相同，不接受任意等义JSON。以此拒绝重复字段、省略Option/default、未知字段及非确定性表示；全嵌套非有限数值须显式拒绝，有限数值不能准确往返时安全失败。此格式只是后续受保护存储的原材料，不是Foundation canonical身份、Ready、持久检查点或真实性认证；解码错误不回显业务正文。

### TDD与可观察验收

先增加一个通过新真实准备interface验证空涨停池固定日期/原报告字节且零外部效果的测试，交主控运行RED。若仅因缺少接口编译失败，明确记为接口RED而非生产事故复现；主控确认后实现并冻结GREEN。其后沿相同公开interface逐项扩展，不能一次写整组假想测试再补实现。

- 空涨停池：精确旧报告字节、business_date、非认证空来源状态；外部adapter若被调用即失败，不能靠环境恰好无配置通过。
- 非空合成来源：真实概念聚类/生命周期输入、持仓匹配、候选状态进入准备结果及报告；旧wrapper和新流程仅发生各自应有的一次效果序列，无重复采集/模型/落库。
- 深度、简化、无模型、数量上限与总览：实际提示词包含独立已知的输入标记；保存原模型返回及失败信息，不虚构成功内容，不改变正文。模型/搜索可通过显式注入的外部协议adapter返回合成结果；不能手工构造整个准备结果来冒充流程验收。
- 原主线新闻检索词生成、原输入优先和宏观回退、可选催化/龙虎榜失败：保存实际分支、时间缺失及降级，不将Unavailable等同VerifiedEmpty。原上游无状态可区分时标Unknown而非捏造错误原因。
- 首次artifact序列化→反序列化：精确报告/模型文本含空白/Unicode仍一致，未知schema/截断拒绝，零外部调用；Debug不含TEST_CODE敏感正文/持仓等标记。不从被测编码函数生成“独立期望”。
- 核心概念/写库/持仓失败：不继续模型/通知，保留错误；固定业务日不由后续时钟改变。已发生效果与首次准备结果分别标识，不以Result<Prepared>冒充已落盘。

旧resolved测试迁移要求：`resolved_chain_facts_persist_match_and_render_without_external_sources`的真实SQLite写入/streak及板块exact/substring/missing断言保留，只将render职责迁至新公开流程；原成功深度/简化/overview及gate_d模型失败保留真实loopback Gemini协议覆盖，不能全换成fake返回值。新准备interface的过渡实现不得在受控非空测试中回落真实legacy I/O；先取缺失interface的编译RED，GREEN前接通所有受控外部效果。

主控基线仅先跑已审计的`pipeline::chain_analysis::tests::empty_limit_up_batch_returns_explicit_empty_report_without_external_calls`，以及按实际影响选择的纯聚类/报告/模型协议测试；不运行整个chain或全仓suite（含环境/全局DB测试）。新组计划为`pipeline::chain_analysis::preparation_tests::`，过滤器以真实声明为准。覆盖关键外部效果次数是防重复合同验收，不用内部helper调用次数替代业务结果。

报告：`.superpowers/sdd/2026-09-11-chain-post-close-recovery/task-1-report.md`。列精确变更、实际命令/RED/GREEN、未执行项、源码冻结点、准备结果哪些只是观察/哪些仍需持久化。主控固定源码后独立Spec/Quality review，重要发现回原作者修复，保留完整Task2–4。

## Task 2: 同业务库检查点、原字节和逐目标发送进度

最新开发状态（2026-09-14北京时间）：已修复龙虎榜历史结果时间校验缺口。70992真实反例后，80508同四项、80583原范围166项回归及87868消费者Clippy均通过；限定独立Spec/Quality通过。源码/日志摘要已核，保留56条测试告警和233条消费者告警，不冒称全仓无告警或完整Task2通过。完整历史身份/Status/重试/提交矩阵、Task2–4/W15–W21/52Unit仍未完成。首批手动包不等待本片，运行根方案尚待确认，本轮未启动或替换monitor。详见[龙虎榜恢复验收](../../push-system/dragon-tiger-recovery-validation-2026-09-13.md)。

前次板块Retry续接验证（2026-09-12）：session50677于11:47:38Z退出0，105项全部通过；编译5m00s/测试65.22s、49告警，585项源码前后/当前与日志SHA一致。真实Retry确认后取消/重开，完整退避后以原request/次数/策略续接，原首组事实不改、跨代引用保持、最终总3RPC与两条原字段BR159均通过，关闭68550反例。6个正常文件修改，原测试/Cargo/固定SQL/codec形状保持。消费者Clippy session64389于11:52:07Z退出0，3m58s、lib202/monitor2告警，585项及日志核一致，较17868无新增/移除诊断种类；仅编译未运行monitor。该冻结点尚未覆盖随后新增的实际授权失败用例，当前结果以上条为准。完整授权/故障/兼容矩阵、错误终结材料、Task2/真实调度和52Unit仍未完成。

前次成功Response提交恢复验证（2026-09-12）：session25581于10:56:05Z退出0，104项全部通过，编译5m56s/测试72.74s、49条告警；585项before/after/当次当前与日志SHA独立一致。真实Concept最终COMMIT争用后回滚，关闭重开换代后只用原Response补最终事务；总RPC仍3、原begin/result及Industry旧审计不改，两条最终BR159的完整原字段/时间/批次均保持。消费者Clippy session17868已于11:01:11Z退出0，编译3m54s，lib202/monitor2告警；585项源码前后/当前与日志SHA一致。仅编译未运行二进制，新增三类诊断已记实施台账，非零告警验收。ConfirmedRetry、错误终结及其他故障/兼容矩阵、完整Task2/真实调度/全部52Unit仍未完成。前次33118真实故障RED已关闭；后续按原完整验收范围继续，不以本例代替全任务。下段为较早缺接口阶段记录。

2026-09-12板块片进度：真实prepare＋原gRPC重试＋owned SQLite首例 session8214 编译退出101，581项源码与日志摘要一致；6处正常接口缺失、1处测试trait导入问题，后者只修导入。v5五类事实SQL已落盘，与批准草稿逐字一致；内存SQLite加载五版SQL退出0，共63个有定义对象。真实迁移/gRPC接点/恢复接口正在实现，尚无修后贯通通过证据。旧v1–v4不改，候选纯unsupported政策不另建状态表；原类型与受保护PreparationFailure均须保留。后续[板块验收矩阵](../../push-system/board-directory-recovery-validation-2026-09-12.md)覆盖授权、错误协议、提交故障、重开及旧版本兼容，不以SQL检查或此前90项替代完整Task2验收。

依赖Task1。实施前按实际接口细化本Task合同和brief，不更改完成目标。受控schema扩展位于既有业务SQLite和独立版本登记；复用Foundation验证/不可变intent、CAS/lease，不修改冻结v1 DDL。包含原准备artifact、报告/chain_daily效果进度、通知目标快照与网络前begin、逐目标结果记录。对每个外部效果开始/结果提交之间的崩溃保留未决；已保存结果只读取原字节。必须接入Task1真实调用位置，不能只有store测试。文件异内容冲突不覆盖，Unknown/部分弱成功/开始后未确认重启零补发；无渠道不伪造attempt。验证真实临时库重开、CAS冲突、损坏/漂移拒绝及所有关键崩溃点。

2026-09-11实际存储核对补充：扩展须由BusinessIntentStore同一连接持有事务，不在rusqlite事务内调用另取Diesel连接的自提交DAO；chain_daily仍保留原upsert语义，不能借P-01整日替换。Foundation独立对象校验不代表GlobalSchema整库catalog已登记，须补批准的扩展认证。现有BusinessExecution仅授权强恢复且production broker拒绝，准备/文件/弱发送需真实窄facade。概念缓存内部逐provider/逐写入、通知循环逐目标begin/result均须实际接线，不可整批执行完才写journal。这些缺口由Task2–4继续实现，不因Task1固定内存结果而消失。

完整Task2实施顺序及验收边界（分项推进，非全量完成；当前进度以上方最新状态为准）：

1. 由现有BusinessIntentStore拥有盘后专用子模块和同一业务连接，独立登记扩展schema/codec。准备前固定run/input/owner，不能等完整pipeline返回后才开始记账。冻结Foundation v1原样保留；临时库重开、半装/定义漂移/未来版本均须拒绝，不能以真实生产库试迁移。
2. 在每次实际外部效果前提交begin，返回后立即保存完整result；概念内部每个provider与缓存写入也必须可区分。chain_daily原upsert、当次生命周期结果和阶段进度由同一个事务提交；崩溃或提交确认丢失时按固定效果身份只读查证，不重跑整段pipeline。
3. 首次完整artifact与Ready绑定必须同事务封存；原record_initial自提交入口不能被外层事务伪装包住。报告文件只用原字节，在受保护目录拒绝符号链接/越界，异内容冲突不覆盖；文件系统与数据库之间的未确认间隙显式保留。
4. 冻结发送目标快照及稳定身份，Custom重复配置项仍是独立目标。在现有发送循环内逐目标执行“持久begin→发送一次→持久result”，结果保存失败立即停后续目标；外层send_report结束后补写整批日志不合格。dispatch已开始的运行重启后不得盲补发未开始的剩余目标，零渠道保持零attempt。
5. 用自有临时库/文件和合成目标验证每个关键崩溃点、旧lease/CAS拒绝、内容冲突及零重复外部效果。弱Accepted/Unknown不推进强完成游标。受保护根、生产认证facade、GlobalSchema扩展认证及真实必达渠道仍须后续明确；工程测试不替代这些条件。

按Task1候选源码60e18ae核对的接线前提：普通来源不可用与持久记录/lease/fence失败必须区分。当前[准备流程](../../../src/pipeline/chain_analysis/preparation.rs)的宏观/候选/搜索会降级，模型深度/简化结果使用`.ok()`；这保留了旧业务政策，但Task2不能把存储或执行权限失败放进同一普通错误通道后继续下一个效果。Task2须以不可被这些可选分支吞掉的停止结果贯穿真实调用点，并验证“begin失败零调用、result提交失败零后续调用”。模型interface目前借用`&self`；持久adapter每次短事务必须先结束连接/可变借用再等待网络，不能为跨await方便开放raw connection或长期持有写事务。

源码依据：[BusinessIntentStore](../../../src/push_foundation/intent_store.rs)、[现有授权范围](../../../src/push_foundation/activation_fence.rs)、[chain_daily DAO](../../../src/database/concepts.rs)、[真实通知循环](../../../src/notification/service.rs)。Task1最终interface冻结后再确定Task2可写文件与精确测试命令；当前不授予扩改生产schema或开启新owner的权限。

### 首片范围：同连接扩展安装与重开校验

Task1限定复审通过后才启动Rust。本片只是Task2的第一个可验证基础，不替代上面逐效果接线、真实通知循环和崩溃合同。

- 首片只允许`src/push_foundation/intent_store.rs`声明子模块/窄入口，以及新增`src/push_foundation/intent_store/chain_post_close.rs`、`src/push_foundation/intent_store/chain_post_close_schema.rs`、`src/push_foundation/intent_store/chain_post_close_tests.rs`。子模块归现有BusinessIntentStore所有，不能独立选择/打开业务路径；若需要固定SQL资产，只能新增同目录`chain_post_close.v1.sql`并由主控先核对登记规则。后续pipeline/fetchers、DAO、notification/file接线逐片扩充允许范围，不提前大面积修改。
- 先定义最小真实职责和安装/验证interface，交主控核对后写首例并冻结。先证明“同一BusinessIntentStore连接受控安装→关闭→重新打开并验证扩展”，再补半装/定义漂移/未来版本/伪造登记与实际定义一起变化的拒绝例。运行时验证不能偷偷安装或修复；缺失扩展是明确未安装，不是空进度。
- 固定独立扩展版本及bundled定义，append-only事实与可变CAS head分别约束；安装一个短事务，版本和对象登记与全部对象一起提交。只在扩展自有表附加保护对象，不改Foundation v1对象/版本/DDL、全应用application_id/user_version或冻结GlobalSchema reference。原Foundation按独立bundled定义仍可验证，不把旧open的自登记一致性检查当成全部认证。
- 首片所有安装试验仅在显式新建的自有临时库上，由合成Test上下文启动。生产安装/运行的许可构造必须保持拒绝，直到真实受保护连接、GlobalSchema扩展认证和owner facade接好；不能把自由传入Test namespace或路径相等当生产授权。不得为方便增加公开raw connection、任意SQL/事务callback或能自行打开路径的新store。
- 首片interface采用`ChainPostClose<'_>`私有借用现有store，提供专用`install_schema`/`verify_schema`和安全receipt；生产`BusinessIntentStore::chain_post_close`固定`ProductionRefused`。成功借用仅来自自己持有临时目录/文件/store且无路径或外部store参数的cfg(test) fixture。连接级FK/recursive_triggers/synchronous/busy_timeout在事务前设置和复核，事务内再核关键值；不能指望事务内切换FK或synchronous。bundled认证临时改变query_only后只恢复进入前能力；失败时结束事务并检查恢复结果，不能悄悄把原只读连接提升成可写。
- 测试fixture用`tempfile`独占目录和显式SQLite连接，装入已校验的固定Foundation DDL及本片所需最少业务表。不得调用忽略路径参数的全局DatabaseManager::init，不调用实际sqlite CLI/provider/环境配置；如需复用现有fixture，先由主控核对其完整效果闭包。独立期望应来自固定字面合同/SQLite实际目录与持久字节，不由被测校验器生成自己的预期。
- 首例只有缺接口时记录编译RED，不人为制造错误；主控审核测试后单队列运行`cargo test --offline --lib <实际声明的完整测试名> -- --exact --test-threads=1`。实现后同名GREEN，随后逐片补故障和重开证据。禁止全库/全仓suite；每次源码冻结且日志与前后摘要保留。首片和后续Task2审查均不作生产迁移或全Unit完成声明。

首例完整名称：`push_foundation::intent_store::chain_post_close::tests::installed_schema_survives_owned_business_database_reopen`。初始扩展缺失须返回NotInstalled且无自动DDL；安装后schema/codec字面版本1，关闭并重开后只验证原记录。核对Foundation字面v1/25个对象及实际catalog，保留application_id/user_version；源码/SQL未确定的bundle摘要不预编造，GREEN实现前提交具体固定定义供主控核对。

主控已核对首片固定SQL：`chain_post_close_schema`、`chain_post_close_objects`两张登记表及六个保护trigger，共八个有SQL定义的对象；只使用扩展自己的表。安装事务先填固定reference的八条定义，最后写唯一header封存，之后插入/更新/删除/替换拒绝。该首版只认证安装完整性，不含run/target/effect表或业务进度，不声称已保存artifact。后续真实业务表必须通过明确的新版本/受控迁移合同加入，不能在同一v1摘要下静默扩对象；版本迁移和保留既有事实的验收仍属于Task2后续工作。

实施者继续一名Rust writer，不执行Cargo/Git/生产命令、不派子代理。主控独占Cargo/Git、公共文档和进度；完整实施报告写本计划私有目录`task-2-report.md`。Task2初始BASE在实际派发前固定；使用本Task摘录brief及只读存储设计，不读取其他计划私有文件。

### 实际进度

Task2 BASE为4da61ad。首例session38308取得缺ChainPostClose/ChainPostCloseError的接口RED后，同连接安装/校验及固定八对象SQL已实现。SQL SHA独立核对为cfaedcafa3bda35942404b874e954a3b88c764a1600e9060a496163721742cb5；session19835单例GREEN：1通过、3405过滤，编译3分02秒/运行0.06秒，Rust/SQL摘要前后未变。当前只有安装登记基础，未实现业务run/阶段/发送恢复。四个相关Rust文件格式差异已修正且检查通过；测试编译45条告警比原43条新增两个未调用入口告警，故障覆盖及完整Task2验收仍在后续。

损坏反例session11762取得实际RED：小写半装、缺保护trigger、未登记附着index、live registry与实际定义一起伪造四例均被拒绝；最后的大写半装被误报NotInstalled，应为SchemaRejected。仅修改目录发现的大小写识别，保留固定定义的精确比较及不自动修复规则；session31606修后2例通过、3405过滤，编译2分52秒/运行0.45秒，源码前后未变。五种损坏的verify/install均拒绝且不改原文件。当前批量补充生产入口拒绝、未来版本类别、只读/既有事务、不可变写保护和真实COMMIT锁冲突验收，不由首例成功代替。

2026-09-12增量：上述五项已补齐，session91731最终7项全部通过、3405过滤，编译3分07秒/运行1.18秒，43条既有测试告警。四个Rust文件格式检查通过，567项源码前后与当前摘要一致，日志SHA复核相符。原只读文件描述符争用fixture在BEGIN出现SQLITE_IOERR_LOCK，沙箱外同例仍失败，不能归因沙箱；只将第二连接改为READ_WRITE打开、仍仅持有BEGIN+SELECT读事务后，实际COMMIT失败分类、完整回滚、重开NotInstalled和后续正常安装均通过。生产实现和固定SQL未为此修改，不以改变断言取得通过，也不据此推断未知底层errno或生产故障。这里完成的是安装基础验收，业务run/阶段/逐目标恢复及完整Task2独立审查仍未完成。

后续业务schema采用显式版本迁移：保留原chain v1八对象和登记行，一次新增可按布局版本追加封存的metadata，此后复用该组表，不为每代复制一套registry。业务对象按实际完整切片定版；验证/重开不得自动升级，不修改Foundation v1或GlobalSchema冻结reference。旧reader须拒绝新布局，迁移须保存既有事实。当前v1→v2迁移已实现并在下述owned临时库测试中通过，不代表生产库已迁移。

持久接线的停止合同已在首个概念provider接入，其余效果仍待：普通来源失败与存储/lease/fence失败用类型区分；可选降级和模型`.ok()`不得吞必须停止。begin确认后在外部await前设置取消保护，只有result持久确认才解除；超时取消留下未决时停止后续效果，Drop不写数据库。原普通失败/15秒宏观、15秒簇搜索、8秒盘后搜索期限保持。model仍可用&self，但store可变借用仅围同步短事务，不跨await；公开prepare保留可识别的结构化停止，而非转成字符串。整个prepare被取消无法返回错误时，以先前固定run及库中原begin/result只读恢复，不承诺存储故障时还能成功补写原因。

下一纵向片已收敛为单用户本地入口、完整原运行context/入口输入、同库lease/CAS和真实单证券概念provider的begin/result。首次取得运行时同时固定7天缓存快照与判定时间，重开使用原快照决定missing，不能忽略命中或因现时缓存变化绕过未决调用；空provider返回也必须先保存原字节，再按业务规则解析。入口绑定真实固定任务映射、不要求外部身份。未迁移的缓存写或后续效果显式停止，不回落到未记账的旧执行器。2026-09-12已新增[5项直接恢复与存储测试](../../../src/push_foundation/intent_store/chain_post_close_v2_tests.rs)，session53084实测编译RED，仅缺本地context/run、codec/阶段、时钟/停止接口；未执行业务断言，不能计作通过。该片现进入实现阶段，尚未验收；临时停止不是最终业务目标，后续仍须接完全部效果。租约须在效果前和外部await返回后重新取时间，另有“无接管但已到期”与“调用跨过有效期”的测试合同。

2026-09-12最新实施覆盖前述“进入实现”状态：首阶段已有非测试local入口、v2完整布局、context/input/cache、任务锁和实际概念raw begin/result及typed stop，测试增至9项。session4875仅两处E0597，修复后session45722合并29例为28通过/1失败（本地9例8通过，原schema7及context/parser/非HTTP准备13均通过），570项源码前后/当前及日志SHA已核。唯一失败为活动lease下显式旧head请求先得到LeaseHeld而非StaleLease；采用“显式旧版本先拒绝；匹配或未提供版本的活动竞争仍LeaseHeld；过期接管必须有匹配CAS”的分类，不放宽执行许可。继续补有限浮点可恢复性、同owner/head的跨run能力拒绝和真实取消反例。未通过本片完整验收，不推进Task3或生产切换。

2026-09-12下一批session3226已终态exit101：32项中31通过、1失败，570项源码前后/当次当前及日志SHA复核一致。旧head分类、直接到期、有限精度输入恢复和真实future取消均通过；唯一自然RED为相同occurrence/owner/generation/head/request、不同run_id的两个owned临时库间Call串用。RunLease与ConceptProviderCall尚缺运行编号，现最小修复为从原持久context绑定并在acquire/resume/begin/result核对；不改测试预期、SQL/codec或去重身份算法。本片仍未全绿，后续缓存/业务写入/模型/报告/通知恢复与Task2整体验收仍待。

2026-09-12修后session64782已终态exit0：同一32项全部通过、3392过滤，2分30秒编译、6.24秒测试、48告警；570项源码前后/当次当前及log SHA一致。只补RunLease/ConceptProviderCall的真实RunId及相应签发/恢复/数据库校验，原失败预期和SQL/codec/hash均未改。现在继续完整概念批次和同库缓存写进度，必须保留旧并发上限6、全部返回后按完成顺序写缓存及坏结果停止规则，同时逐真实返回立即持久化；不能以串行化规避多个在途调用的版本绑定。此为Task2局部GREEN，完整恢复与Task2独立审查仍待，不推进生产切换。

Task2当前增量（2026-09-12）：完整概念批次新增贯通例已通过。首次合批session60795为32/33，修复v2意外允许第二请求的实际版本边界后，session49295同33项全过（48条告警，源码/日志摘要均核对）。只允许已验证v3布局运行完整批次，v2首阶段限制与原断言/SQL保持。完整六并发/乱序、普通错误收齐、取消/提交失败、缓存与迁移回滚及持久关联补验仍在Task2，不提前进入Task3生产接线。

## Task 3: 实际定时器、启动恢复和窗口资格

最新Task2前置证据：session76721 于07:34:22Z退出0，90项全部通过、47条告警，579项源码与日志摘要已核。新审计同事务追加及19项新旧审计测试、原71盘后恢复保持；消费者Clippy1348已于07:41:10Z退出0（lib199/monitor2告警），579项源码及日志摘要一致，较31234无新增或移除诊断种类；仅编译、未运行monitor不运行。板块schema/全链准入、RPC/后续来源模型/报告发送仍待，Task3尚未开始，发送失败误封日不能据此关闭。

此前Task2前置证据：session85939同71项全部通过，47条告警，578项源码前后/当次当前及日志摘要一致。固定聚类材料/原图关联、同库业务故障恢复保持；候选停止/普通降级、真实v4旧事实迁移/COMMIT回滚与重开补验已通过。同版本消费者Clippy31234退出0，保留lib199/monitor2告警及两类新增诊断；其余来源/模型/报告/发送恢复仍待，Task3仍未开始，不将测试数量当Unit完成数。

前置Task2最新验证（2026-09-12）：完整概念批次与关联修复后session70253同45项全部通过，47条既有测试告警；旧停止类型与真实存储cause同时保留，跨事实版本/请求/owner/代次/时间矛盾被拒绝，合法跨代缓存恢复保持。该结果只覆盖已接概念/缓存片；旧事实迁移与独立SQL故障补验、阈值/聚类/后续来源模型/报告通知仍在Task2，不据此提前接管定时器或关闭Task3–4。

依赖Task2及单用户本地运行context的实际提供接口；修改app和monitor真实入口，不借盘前/CLI/R03的运行/完成编号。新工作在交易日15:30≤当前时间<15:35检查，进入前重新读时钟；固定业务日与自然日分开，窗口外仅恢复已有事实。替换仅凭Ok封日的完成推导，返回准备/保存/弱观察/权威完成不同事实。本地显式配置与任务锁须接在真实路径，不能用测试fixture替换旧路径或静默切换执行者；不等待外部身份认证。以实际timer消费的interface测延迟过窗、跨日同业务日、重启、部分发送、未决恢复和不重跑模型，保持CLI/盘前黄金外部行为。

## Task 4: 强完成cursor、六门禁与切换材料

依赖实际本地注册、必达渠道/authority及W15/W16本地就绪检查与任务锁接线，不再依赖复杂可信身份认证。复用已有强投递/重验/finalizer，不将弱观察提升；同业务事务提交独立cursor、Completed与稳定事件。接受后仅恢复最终化，Unknown隔离、Rejected只按显式授权新attempt。完成unit/failure/crash/shadow/dedup/rollback六门禁；准备绑定实际Unit/build/generation/窗口/evidence的切换/回滚命令。部署迁移、实际渠道调用和执行者切换仍须用户明确授权，测试不能替代自然观察或远端接受。全局W18–W21按单用户调整后的范围及余下51 Unit仍保留，不以本Unit覆盖。

## 回退与记录

Task1仅局部Rust提交可回退，无生产状态变更。Task2新增事实只保留不删除、不降schema；回退代码必须明确兼容读和旧writer排空边界。Task3/4操作性回退须先核实当前owner和未决责任，不能git回退代替生产回滚。完成进度、测试证据和未决项同步docs入口。
