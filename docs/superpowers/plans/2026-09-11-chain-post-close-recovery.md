# 盘后产业链：固定准备、持久恢复与真实定时器接管

日期：2026-09-11。初始源码42ce098，隔离分支codex/push-reliability-20260905。状态：执行中，尚未完成。

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

依赖Task1。实施前按实际接口细化本Task合同和brief，不更改完成目标。受控schema扩展位于既有业务SQLite和独立版本登记；复用Foundation验证/不可变intent、CAS/lease，不修改冻结v1 DDL。包含原准备artifact、报告/chain_daily效果进度、通知目标快照与网络前begin、逐目标结果记录。对每个外部效果开始/结果提交之间的崩溃保留未决；已保存结果只读取原字节。必须接入Task1真实调用位置，不能只有store测试。文件异内容冲突不覆盖，Unknown/部分弱成功/开始后未确认重启零补发；无渠道不伪造attempt。验证真实临时库重开、CAS冲突、损坏/漂移拒绝及所有关键崩溃点。

2026-09-11实际存储核对补充：扩展须由BusinessIntentStore同一连接持有事务，不在rusqlite事务内调用另取Diesel连接的自提交DAO；chain_daily仍保留原upsert语义，不能借P-01整日替换。Foundation独立对象校验不代表GlobalSchema整库catalog已登记，须补批准的扩展认证。现有BusinessExecution仅授权强恢复且production broker拒绝，准备/文件/弱发送需真实窄facade。概念缓存内部逐provider/逐写入、通知循环逐目标begin/result均须实际接线，不可整批执行完才写journal。这些缺口由Task2–4继续实现，不因Task1固定内存结果而消失。

## Task 3: 实际定时器、启动恢复和窗口资格

依赖Task2及可信运行context的实际提供接口；修改app和monitor真实入口，不借盘前/CLI/R03 identity。新工作在交易日15:30≤当前时间<15:35检查，进入前重新读时钟；固定业务日与自然日分开，窗口外仅恢复已有事实。替换仅凭Ok封日的完成推导，返回准备/保存/弱观察/权威完成不同事实。无认证不得绕过构造器或静默启用新owner；上线前旧路径不被测试fixture替换。以实际timer消费的interface测延迟过窗、跨日同业务日、重启、部分发送、未决恢复和不重跑模型，保持CLI/盘前黄金外部行为。

## Task 4: 强完成cursor、六门禁与切换材料

依赖真实注册、必达渠道/authority及W15/W16当前认证/fence接线。复用已批准强投递/重验/finalizer，不将弱观察提升；同业务事务提交独立cursor、Completed与稳定事件。接受后仅恢复最终化，Unknown隔离、Rejected只按显式授权新attempt。完成unit/failure/crash/shadow/dedup/rollback六门禁；准备绑定实际Unit/build/generation/窗口/evidence的切换/回滚命令。生产身份/迁移/调用/owner切换必须有具体批准，测试不能替代自然观察或远端接受。全局W18–W21和余下51 Unit仍保留，不以本Unit覆盖。

## 回退与记录

Task1仅局部Rust提交可回退，无生产状态变更。Task2新增事实只保留不删除、不降schema；回退代码必须明确兼容读和旧writer排空边界。Task3/4操作性回退须先核实当前owner和未决责任，不能git回退代替生产回滚。完成进度、测试证据和未决项同步docs入口。
