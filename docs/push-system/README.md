# 推送系统文档入口

状态：`PROVISIONAL`。本批输入冻结日期为 `2026-09-06`。源码接线、历史统计、原工作区文档和拟议设计必须分开阅读；它们不等于部署证明、远端 `TransportAccepted` 或用户已读。

## 当前开发入口（更新至2026-09-16北京时间；运行记录按各自日期阅读）

- [账户截图与推送错配修复](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/account-snapshot-push-mismatch-2026-09-16.md)：最新持仓已导入；日期/来源、横幅、估值绑定和T-02整条消息开发验收完成，35项回归、monitor编译及独立审查通过；尚未部署，不能据此认定用户已收到修正消息。

- [午盘/日终复盘与归因四项](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/review-attribution-call-chain-2026-09-16.md)、[交易/风控五项](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/trade-risk-call-chain-2026-09-16.md)、[新闻/虚拟/盘后侧路五项](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/news-virtual-side-routes-call-chain-2026-09-16.md)：最后14项已复核，23/24/25项快照实核；52个Unit均有主要调用链/完成门静态记录，不是整仓逐行审查或迁移完成。
- [gRPC下游交接（原项目根）](/Users/zhangzhen/Desktop/Quant/stock_analysis/grpc_handoffs/README.md)：独立agent按用户要求整理原13项，主控新增公告/大宗交易2项，当前15项数据问题/验收项；运行事实、协议待确认、投影丢失、业务口径及账户来源分别列证据，不作已修复声明。
- [市场/板块四Unit（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/market-sector-call-chain-2026-09-16.md)与[T0/CloseCall/PaperTrade三Unit（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/ticket-t0-close-call-paper-trade-call-chain-2026-09-16.md)：21/20项快照实核，该片完成时38/14，当前52项静态覆盖见顶部；不等于实现、部署或迁移完成。
- [本机gRPC当前故障与数据充分性（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/grpc-data-readiness-assessment-2026-09-16.md)：00:47日志描述符耗尽/数据库打开失败，两次健康连接超时；未取得最新能力/批次、不自动重启，也不据此宣布所有External来源不可用。
- [模型/搜索/首次报告后继准备（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/chain-models-search-recovery-preparation-2026-09-16.md)：9项源码/合同摘要已核，真实调用序列及窄同库恢复方向明确；待完整Macro验收后实施。
- 当前开发：[盘后Macro恢复进度（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/chain-macro-recovery-progress-2026-09-14.md)、[完整Macro实施合同](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/chain-macro-full-implementation-2026-09-15.md)与[实施计划](../superpowers/plans/2026-09-14-chain-macro-recovery.md)。首来源Task2的50不同修正用例/消费者和独立复审已通过；完整Task3首条贯通已取得真实行为RED，正在实现共享runner/v12/真实prepare，不重做已验收片。完整Macro及后继业务仍未完成，未部署本Macro改动。

- 最新实际运行版本：[扫描修复发布验收（原物理 docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/releases/monitor-scan-release-2026-09-14.md)。PID 92715 已于12:15:12完成启动对账、12:15:13取得真实飞书回执；扫描Task1的8+14定向测试、独立审查、消费者检查与release已完成。前一PID30370已优雅退出；下面旧部署/尚未部署条目保留其历史时间含义。SQLite生产修复、盘后后续恢复与全部52Unit仍未完成。

- 最新实际部署优先读[原物理 docs 的启动验收](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/releases/monitor-start-2026-09-14.md)：完整开发树构建的优化 release 已运行，非下文两文件临时参考包。生产根/正文根修复、真实飞书回执已验证；完整迁移未完成。现在继续[运行期阻塞与连接校验修复](../superpowers/plans/2026-09-14-monitor-runtime-reliability.md)，隔离开发，不停止现有 monitor。下文“尚未启动/范围待确认”只对应各自历史切片。

- 当前优先交付[首批手动推送独立源码包](releases/manual-push-2026-09-14/README.md)：仅两文件，原目录HEAD a673043的干净参考树已通过正反向检查、10项手动测试、7项原调度测试及非测试dev可执行文件构建；三次各834项输入和日志摘要核对一致，参考制品SHA3d4e494f…6967f。尚未部署：原release构建源待认证，临时编译根会绑定另一份数据库/实例锁/审计，不能直接复制替换。原目录160个冲突及旧二进制保持。详见[参考验证与切换缺口](releases/manual-push-2026-09-14/reference-validation.md)；盘后全链恢复不再作为此首包前置，[独立交付计划](../superpowers/plans/2026-09-14-first-incremental-release.md)保留正式制品、回滚和启动验收。

- [CLI replay/single/summary三个Unit（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/cli-replay-single-summary-call-chain-2026-09-16.md)：23项快照已核，三分析入口及历史再发的调度、输入、弱回执和通知后审计边界补齐。该片完成时31/21，当前累计见顶部，不等于迁移完成。
- [业绩/评级三个独立Unit（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/earnings-analyst-call-chain-2026-09-16.md)：26项源码快照经主控核验，报告期/issuer、空评级报告、发送前状态推进及保守恢复边界已记录。该片完成时28/24，当前累计见上条。
- [公告/D01/新闻催化三个独立Unit（原物理docs）](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/announcement-d01-catalyst-call-chain-2026-09-16.md)：自动/手动/恢复、claim/冷却及通知后失败已核；该片完成时25/27，当前累计见上条。
- [R03产业链复盘三个独立入口](review-r03-call-chain-2026-09-14.md)：自动/手动新消息均被无条件AccountMetricsIncomplete阻断，启动只恢复既存信封；Terminal不等于Delivered。该片完成时22/30，当前累计见上条，不等于迁移完成。
- [R13名单核对 / A10催化复盘入口与恢复缺口](review-r13-a10-call-chain-2026-09-13.md)：已核通知后业务保存裂缝、R13前排名单错位与部分成功、A10历史来源和--push窗口差异。该片完成时19/33，当前累计见上条，不等于迁移完成。
- [R11持仓复盘四入口与恢复缺口](review-r11-call-chain-2026-09-13.md)：空持仓首次可投递，但dispatcher复用Delivered时零快照被拒绝；历史来源绑定与通知前AI文件效果待修。该片完成时为17/35，当前累计见上条。

- [龙虎榜真实效果与恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)：逐次RPC、自然日事实、同库journal/审计及公开prepare真重开已取得局部通过证据；最新F2损坏反例、修复和整体验收边界见下方状态，不把session重建当SQLite重启验收。

- [龙虎榜复盘四入口调用链](review-r04-call-chain-2026-09-13.md)：自动/手动/补推/启动恢复共用原业务日通知claim，但不与盘后chain来源共用完成权；自动manual override及补推附带侧路已按源码记录，不计算迁移完成率。
- [R09来源榜单复盘四入口](review-r09-call-chain-2026-09-13.md)：实际单次RPC仅传date，两份limit/filter证据由本地构造并校验返回行；来源硬失败发生在decision之前的父任务恢复缺口仍待修复。此片完成时为16/36，当前累计见顶部最新追链条目，不等于迁移完成。
- [R07明日观察 / R08事件日历四入口调用链](review-r07-r08-call-chain-2026-09-13.md)：R07同日手动/自动均等21点，四源正文不等于统一来源快照；R08为Rolling、CFFEX硬门，来源失败先于decision的持久任务缺口保留。此片完成时为15/37，当前累计见上一条。

- [逐步替换：首批候选与切换条件](incremental-replacement-readiness-2026-09-12.md)：优先准备手动初始化/失败结果的两文件历史切片；明确竞价后续依赖与持仓身份兼容风险。尚未生成发布制品或切换monitor，不把整个dirty分支当首批包。

- 最新开发状态（2026-09-14北京时间）：已修复龙虎榜历史结果时间校验缺口。70992真实反例后，80508同四项、80583原范围166项回归及87868消费者Clippy均通过；限定独立Spec/Quality通过。源码/日志摘要已核，保留56条测试告警和233条消费者告警，不冒称全仓无告警或完整Task2通过。完整历史身份/Status/重试/提交矩阵、Task2–4/W15–W21/52Unit仍未完成。首批手动包不等待本片，运行根方案尚待确认，本轮未启动或替换monitor。详见[龙虎榜恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)。

- 前次板块Retry续接验证（2026-09-12）：session50677于11:47:38Z退出0，105项全部通过；编译5m00s/测试65.22s、49告警，585项源码前后/当前与日志SHA一致。真实Retry确认后取消/重开，完整退避后以原request/次数/策略续接，原首组事实不改、跨代引用保持、最终总3RPC与两条原字段BR159均通过，关闭68550反例。6个正常文件修改，原测试/Cargo/固定SQL/codec形状保持。消费者Clippy session64389于11:52:07Z退出0，3m58s、lib202/monitor2告警，585项及日志核一致，较17868无新增/移除诊断种类；仅编译未运行monitor。该冻结点尚未覆盖随后新增的实际授权失败用例，当前结果以上条为准。完整授权/故障/兼容矩阵、错误终结材料、Task2/真实调度和52Unit仍未完成。

- 前次成功Response提交恢复验证（2026-09-12）：session25581于10:56:05Z退出0，104项全部通过，编译5m56s/测试72.74s、49条告警；585项before/after/当次当前与日志SHA独立一致。真实Concept最终COMMIT争用后回滚，关闭重开换代后只用原Response补最终事务；总RPC仍3、原begin/result及Industry旧审计不改，两条最终BR159的完整原字段/时间/批次均保持。消费者Clippy session17868已于11:01:11Z退出0，编译3m54s，lib202/monitor2告警；585项源码前后/当前与日志SHA一致。仅编译未运行二进制，新增三类诊断已记实施台账，非零告警验收。ConfirmedRetry、错误终结及其他故障/兼容矩阵、完整Task2/真实调度/全部52Unit仍未完成。详见[板块验收矩阵](board-directory-recovery-validation-2026-09-12.md)。

## 历史进展与证据索引

以下按各次冻结版本保留过程证据；“待验”“最新”等措辞只对应该条记录当时的状态，当前结果以上方板块进度为准。

- 最新盘后验证：session76721 于07:34:22Z退出0，90项全部通过（审计19+原盘后71），47条告警；579项源码前后/当次当前与日志摘要已核。BR159同事务追加、真实SQL/COMMIT故障回滚、新旧writer/reader、跨能力/来源前态、坏尾/缺表/只读拒绝均通过，原盘后恢复保持。消费者Clippy1348已于07:41:10Z退出0（lib199/monitor2告警），579项源码及日志摘要一致，较31234无新增或移除诊断种类；仅编译、未运行monitor；板块schema/全链准入、实际RPC与后续来源/模型/报告发送仍待。以下为历史验证点，测试数不是Unit迁移数。
- 最新盘后验证：session85939于06:43:43Z退出0，71项全部通过、47条告警，578项源码前后/当次当前及日志SHA一致。新增真实prepare的候选硬停止与普通Unavailable政策、实际v3→v4旧事实保持、迁移COMMIT整体回滚/重开重试、异名index/trigger拒绝不修复均已验证。首轮E0603仅为测试引用私有类型，修正测试私有cause后通过，生产/SQL/codec未改。[v4维护测试](../../src/push_foundation/intent_store/chain_post_close_v4_migration_tests.rs)只使用自有临时库。同一源码消费者Clippy31234退出0（lib199/monitor2告警，较75032新增两类诊断）；后续来源/模型/报告发送、真实timer、强完成及52个Unit仍未完成，以下为历史阶段证据。
- 当前聚类修后验证：session43450于2026-09-12 06:12:16Z退出0，同67项全部通过、47条告警；577项源码前后/当次当前与日志摘要均独立核对一致。上一轮三个反例已关闭：业务应用在同事务内绑定原完整概念图，两条父子事实关系拒绝代次倒退和同代执行者矛盾，合法跨代恢复保持。全部67项预期及冻结SQL/codec未改。下一组继续候选硬停止/普通降级和v4旧事实迁移、真实提交回滚维护；后续来源/模型、报告/逐目标发送、实际timer及强完成仍待。以下session是各自历史冻结点的证据，不能把测试数当52个Unit迁移数。
- 聚类维护最新结果：session28463同67项64通过、3失败，47条告警；577项源码前后/当前及日志摘要一致。三个反例暴露业务应用前未绑定原完整概念图、父子事实代次与同代执行者关联校验缺口；正在最小修复，不能将下一条原53项通过当本轮全绿。其余新增配置/完整图/跨代/空与重叠簇、真实业务SQL和COMMIT回滚/重开维护已通过。只使用自有临时库，不是生产事故或生产切换。
- 最新盘后聚类验证：session79018于2026-09-12 05:18:00Z退出0，53项定向测试全部通过、47条告警；576项源码前后与当前摘要、日志SHA均核对一致。新增实际prepare贯通了固定阈值、原聚类/别名/孤立股票、chain_daily同库应用及原生命周期重开恢复，现时业务修改不被恢复覆盖。聚类边界/故障/关联损坏和v4旧事实迁移补验仍待；后面的来源/模型、报告/逐目标发送、真实timer与强完成状态尚未完成。下列各session为历史阶段证据，53项不是52个Unit迁移完成，消费者Clippy仍是v4改动前的证据。
- **最新范围调整：[单用户本地模式](single-user-local-scope-2026-09-11.md)**。按用户决定，本次取消复杂可信身份、角色授权/双人审批及外部身份发行方前置；保留任务锁、运行编号、同库事务、原字节恢复和防重复推送。旧认证缺口文档保留为历史，不再据此阻塞本地接线；本地准入正在开发，尚未验收，不冒称生产已切换。
- [盘后产业链持久恢复实施计划](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)：基线42ce098，Task1最终源码40b0a63。固定事实/模型/候选证据、版本化原字节封存已接真实准备入口，旧回归已迁移，生产/旧持仓回归共用匹配实现。修后19项回归、另两个真实SQLite单例、CLI/monitor编译及格式检查通过，独立初审与限定修复复审完成；Task1工程验收通过，详见[实施进度与验证边界](implementation-chain-preparation-2026-09-11.md)。2026-09-12 Task2已实现单用户本地入口、固定输入/缓存、任务锁及首个概念查询的持久记录；跨运行凭据绑定修复后session64782合并32项全部通过，保留48条编译告警。此前旧版本任务锁分类、真实取消、输入有限精度、7项安装基础及首阶段结果重开不重复查询均有证据；后续完整概念批次/缓存写、业务落库、原报告和发送恢复仍待。保持一名Rust实施者、主控单Cargo队列；尚未改定时器，不代表误封日已修复或生产已切换。
- 盘后完整概念批次/同库缓存进度首轮贯通验证通过：session60795发现旧v2第二请求被意外放行，恢复“v2仅首阶段、v3才完整批次”的实际布局边界后，session49295同33项全部通过，保留48条告警。原断言/SQL未改，新批次原结果与缓存进度重开后零查询/零重写已有实测。后续补验进度见下一条；此33项只对应该冻结版本，不代表后续新增测试或完整Task2验收通过。
- 最新补验session14534：40项全部通过、47条告警。覆盖全命中完整图/更新时间保持、8请求最多6在途及乱序逐结果持久、普通错误/空raw收齐后按顺序停止缓存、多在途取消和读锁故障停止、缓存业务行/事实/head回滚、三态partial恢复。此前测试错误的原因读取和Result解包均已修正，生产脱敏未改。读锁回滚不冒充公开错误已有底层operation诊断；该诊断、持久关联拒绝、独立SQL写故障与迁移补验仍待，整个Task2及生产切换未完成。
- 后续事实关联反例session20424：45项38通过、7失败，47条告警。两个故障例未保留底层存储cause；五个保持原schema的异常关联例被读取错误接受（code/ordinal串用、跨事实版本碰撞、owner/代次/时间矛盾）。修后证据见下一条；测试只使用自有临时库，不是生产事故。原40项通过属于上一冻结版本，不能代替本轮新增断言验收。
- 最新修后session70253：同45项全部通过，47条告警。仅facade补真实cause保留和原请求/结果/缓存的关系校验，原停止分类、SQL、codec及测试预期不变；合法跨代缓存恢复仍通过。两个真实读锁故障现已直接核到commit操作类别，五个异常关联例均拒绝。继续旧事实迁移/v3迁移回滚与封存拒绝、独立SQL写故障补验；不代表完整盘后流程、Task2或生产切换完成。
- 迁移维护组合批session58384：52条定向测试50通过、2失败，47条告警。独立cache SQL写入失败/cause与回滚、未来/孤立/未封存拒绝、封存写保护及异名index/trigger独立拒绝已通过；两例被测试快照错误列名阻断，尚未到实际迁移/迁移COMMIT断言，正在只修测试查询。52是用例数，不是52个推送单元已迁移。
- 最新session44708：修正测试快照列名后，同52项全部通过、47条告警；573项源码前后/当前及日志摘要一致。两份真实旧v2运行无损迁移、旧reader拒绝、重开零provider，以及实际迁移COMMIT失败完整回滚/重开仍v2/后续正常迁移均已执行通过，生产实现与冻结SQL未为此次修正改变。当前继续消费者编译和聚类/业务落库恢复；整个Task2、定时器误封日与生产切换仍未完成。
- 同一源码消费者检查session75032退出0，lib、stock_analysis及monitor编译通过（未运行二进制）；库197条、monitor 2条告警。与前次97458按告警标题和源码路径比较无新增/移除诊断种类，不称全仓零告警。聚类/chain_daily下一片开始测试先行，旧52项证据不代替后续新增代码验收。
- 聚类首例session87295取得9个缺接口编译错误，574项源码前后/当前及日志摘要一致，未执行业务断言。新增测试要求真实prepare固定阈值/原成员别名与孤立股票、chain_daily原upsert/10自然日生命周期及重开零重写。v4存储候选已纠正配置与概念图的先后关系、历史事实版本与当前head的区分；聚类恢复正在实现，尚未验收，不将上一冻结点52项通过说成当前新增测试已通过。
- [真实通知逐目标结果实施记录](implementation-notification-attempt-observation-2026-09-11.md)：初版bd143fe、最终de38876；旧send实际委托逐目标观察，保留Custom重复目标及任一弱成功投影，false/Err保留Unknown。原修后16项定向测试与独立复审通过，本前置Task完成。2026-09-12另修普通飞书长报告截断，session76293已验证原文完整分片与旧通知兼容，预算边界继续补验。[盘后持久恢复设计](chain-post-close-recovery-design-2026-09-11.md)仍保留真实timer、同业务库进度、重启恢复与强完成cursor；不能说误封日或完整Unit迁移已完成。

当前审计基线：后续蓝图发现的MU-auction-volume旧摘要已由当前审计Task2在047b4ab修正；该批真实draft0、strict仅七项发布条件、610项只读与独立Spec/Quality均通过，新增问题已关闭。新版蓝图已消费最终SHA，不混用审计初版ff94eca身份。另撤回“v18/v19实际仅九份文件”的结论：Git跟踪16份，其中九份属于固定source catalog，另七份已完整读取并单列原文证据，不扩张冻结来源权威。

2026-09-10：完整[当前架构蓝图](../architecture/current/Project_Architecture_Blueprint.md)已在c1e24d0交付Markdown，覆盖18节、65/102/52精确身份及16份设计来源，独立Spec/Quality Approved。双目标源码abeabf6通过55项/1128断言及独立审查；[当前离线蓝图](../architecture/current/Project_Architecture_Blueprint.html)和[RFC离线页面](push-system-implementation-rfc.html)实际生成、重复/只读门禁及浏览器验收通过，制品已在3283eaf纳管。最终三项收尾fa991e2通过新增2项/14断言和唯一限定复审，3/3关闭；本蓝图计划完成。具体摘要/命令/发布阻断见[实施记录](implementation-current-blueprint-2026-09-10.md)，不代表完整运行时或生产迁移完成。

- [统一文档门禁实施记录](implementation-unified-document-checker-2026-09-09.md)：最终源码7a150b2，限定复审Approved；enum失败漏strict/无ID重复错误、root参数及额外别名全部关闭。修复后定向2/65、1/17、3/28通过；实际draft107/strict112和598项只读证明重新取得。首次TDD过程例外及初审描述更正均保留，不冒称当前draft或远端CI通过。合同见[计划](../superpowers/plans/2026-09-09-unified-document-checker.md)。
- [强制当前源码审计实施记录](implementation-current-source-audit-2026-09-09.md)：初版c2e33a2、最终修复ff94eca；历史/current两层同批接通，548文件/250声明、65 kinds/102 producers/52 Units身份保持。Catalog整套48/620及最终定向通过；checker原20例有1个测试预期错误，修正后单例10断言通过。独立初审发现的集合竞价摘要遗漏已修复，限定复审Approved、无开放问题。修复后真实draft0、strict仍为6项provisional+提交前dirty，610项只读证明通过；仅此审计Task完成，不代表发布或迁移完成。[计划](../superpowers/plans/2026-09-09-current-source-audit.md)保留全部合同。
- [当前Unit摘要后续修复](implementation-current-source-audit-2026-09-09.md#task2-unit层残余说明纠正)：047b4ab仅改三current制品4+/4-，修复主循环/dispatcher内部推进与双次采集旧文案；独立Spec/Quality通过，原字节绑定/正式派生和实际树验收齐全。此条为最终current制品入口，历史规范不变。
- [当前蓝图与双目标HTML计划](../superpowers/plans/2026-09-09-current-blueprint-offline-html.md)：两Task、实际产物及最终收尾均已完成，独立审查和限定复审通过；新路径不覆盖冻结蓝图。
- [W15当前记录查询实施记录](implementation-readiness-current-query-2026-09-10.md)：初版c737110、孤立分支修复14def95；修复后8项query/12项store及格式/Clippy通过，独立初审质量Approved、限定复审问题关闭。本内部任务完成，初版过程证据与activation直测Minor仍保留。复用v3/store/完整集合重读，拒绝历史head与查询期间漂移；仅内部候选入口，不冒充同一认证快照的最终消费者。[计划](../superpowers/plans/2026-09-10-readiness-current-record-query.md)保留完整认证接线目标。
- [W15/W16 → P-02认证与运行时接线缺口](runtime-auth-integration-gap-2026-09-10.md)：按8ad4f9f及独立只读核对，明确生产身份入口始终拒绝、生产broker构造始终拒绝、context有效输入仍只有测试fixture三处断点。区分尚未实现的认证代码与外部平台/受保护根配置，不把原始UID、候选hash、PAM跳过Ok或放宽构造器当完成；本轮仅补接线证据，没有改变Rust或现有推送。
- [W18操作员请求入口实施记录](implementation-operator-request-intake-2026-09-10.md)：初版409dbaf、最终9f35ab1；修复后16项定向测试/格式通过，Clippy退出0但保留29条dead_code Minor，限定复审两项关闭且无新重要问题，本输入Task完成。严格wire和命令身份不认证原始引用；真实批准、独立审计、执行/响应仍必需。[计划](../superpowers/plans/2026-09-10-operator-request-intake.md)保留完整后续目标。
- [当前代码审计增量](current-code-audit-delta-2026-09-09.md)：固定aef7972；107条原始漂移已归类，新增79文件含46生产/33测试，9+29候选已作为后续正式current审计输入。最终current材料见上方047b4ab后续修复，原增量文档保留其调查时点和范围，不改旧RFC/WBS/runtime目录。
- [当前蓝图规模、调用链与运行边界核对](current-blueprint-inventory-2026-09-09.md)：全仓594个Rust文件/445884行、62个公开顶层模块，与push审计548项口径分开；补核认证/DB/配置/CI、CLI与数据平面、两套Foundation存储、依赖与测试证据边界。四时段主归属按规范10/6/21/28计数，不沿用旧蓝图小标题。targets仅静态候选，无metadata或生产验证，不代表新版蓝图/第二HTML已完成。
- [RFC 离线HTML构建](implementation-offline-rfc-html-2026-09-09.md)：最终源码ca1b581；23项测试/676条断言通过，独立限定复审Approved，原3项及段落边界回归均关闭。实际浏览器4图成功/1图回退、点击与注入阻断、全屏和页面0外部请求通过；[离线页面](push-system-implementation-rfc.html)已纳管。保留官方发行JS原字节及30条空白告警，不覆盖八份冻结输入，不代表双份HTML/统一checker/CI或当前证据目录已完成。
- [旧库升级与恢复审计接续](implementation-durable-upgrade-2026-09-08.md)：源码77cc3bc；正式升级/提交、原rowid与数据保持、正确接续/终态、重复恢复/实例重开及坏FK完整回滚，最终67项合批、静态检查及独立规格/质量审查通过。已被旧迁移重排的v5–v9库保留独立兼容任务，不代表全部审计兼容或全项目完成。
- [Durable 运行期版本防护](implementation-durable-runtime-schema-guard-2026-09-10.md)：源码8ee1e13；初始版本漂移、callback前/写锁后二验、事务回滚、read后验和final open检查已实现，9新+14旧定向测试及静态检查通过，独立Spec/Quality Approved，本Task完成。报告摘要Minor已纠正并只读复核，保留43测试/188Clippy旧告警；不改schema版本，不代替旧writer排空或历史顺序兼容。
- [手动推送健康准备与失败结果](implementation-manual-push-bootstrap-outcome-2026-09-10.md)：初版de990c8、最终41e7762；先准备真实banner，部分失败返回Err并由原CLI退出2，盘后A-01健康失败仍继续独立A-10，P-01 owner不变。两个真实RED后实现，初审A-01测试接口问题修正为date-only，修后10项回归/静态检查与限定复审通过，旧7项scheduler证据保留。Clippy无新增告警；全仓fmt旧差异明确保留。本局部任务完成，不改变时间窗口或宣称生产已使用新代码。
- [持仓计划同快照、同行情批次来源接线](implementation-holding-plan-frozen-source-2026-09-10.md)：源码112ff8f；实际manual/periodic共用准备固定一份持仓、按其代码请求行情并保留完整batch evidence，单次本地时间驱动正文/业务日/本地observed_at。来源实际RED后修复，27项最终定向测试及静态检查通过，独立Spec/Quality通过；既有告警Minor保留。日表与durable统一、有效修订/再次发送资格及来源认证仍待，不是完整Unit迁移或生产切换。
- [恢复分类不确定投递读取修复](implementation-recovered-uncertain-read-2026-09-08.md)：源码a2429b3、修复fb55d32/61e9d16；真实过期恢复贯通Generic/P01及SLA/指标。自环与跳链均实际RED后修复，最终55项/静态检查/限定复审通过；无盲重发或状态提升，不等于完整Q39/W19完成。
- [CI 映射式触发识别修复](implementation-ci-mapping-trigger-2026-09-08.md)：源码80d0fb2；95项定向测试、614条断言通过，独立复审已关闭排除项取消全部正向匹配漏洞；识别当前真实CI触发格式，不改CI配置，也不代表HTML/统一checker/实际CI已交付。
- [52个迁移单元的当前完成证据](remaining-migration-evidence-2026-09-08.md)：2026-09-13补齐R09四入口后，16个Unit已定位实际旧业务入口及主要完成门，36个尚未逐链完成；原15/37、13/39等历史口径保留，不以注册表、调用链或测试数计算迁移完成率。
- [盘前与CLI产业链调用链](chain-preopen-cli-call-chain-2026-09-13.md)：核对09:05–09:15/自然日日期门、CLI分派与dry-run未消费、最近已完成交易日、财联社20取15、报告先保存和弱发送bool。盘前失败误封日/CLI失败仍正常返回/同名覆盖均为源码条件反例；逐目标弱观察已经存在，持久恢复与实际切换未完成。
- [集合竞价候选单元调用链](auction-candidates-call-chain-2026-09-10.md)：源码409dbaf；A-02/P-05分时成功与同轮双bool外门不匹配，P-05在主卡发送前推进快照并忽略失效通知结果。仅源码反例与后续接线约束，不是生产复现或已修复，不扩大当前P-02量能诊断任务。
- [盘中连板单元调用链](limit-boards-call-chain-2026-09-10.md)：源码1931014；上游主力净流None与下游Some过滤导致当前选集为空；正式接源后还需解决发送前封口、Top10前推进集合及三形态共享冷却。仅静态分析，尚未修复或生产验证，不混入当前P-02诊断任务。
- [盘后产业链单元调用链](chain-post-close-call-chain-2026-09-10.md)：源码20f215d；通知false/异常被mode吞成Ok后封日，跨自然日可重复同业务日，报告先落盘且同名覆盖，多渠道bool不等于强回执。保留快讯回退与龙虎榜独立日期/降级边界；只读分析，未修复或生产复现。
- [盘中持仓计划三入口调用链](holding-plan-call-chain-2026-09-10.md)：原源码8daa8bf的手动banner/错误退出问题已由41e7762修复；重复持仓读取/行情证据丢失已由112ff8f修复并独立审查通过。周期日表与durable非原子、manual/startup不维护同一日表、来源认证与有效修订资格仍待。既有裁决要求真实修订不能被日级展示名吞掉，但不授权仅时间戳变化自动重发；不是完整Unit迁移验收。
- [P-02 来源观察保留实施记录](implementation-auction-source-observation-2026-09-08.md)：本计划完成，20项lib与51项monitor回归、Clippy及独立评审通过。实际采集/名称分片/审计回执保留到竞价tick消费，旧投影兼容；非空池缺量比不称VerifiedEmpty，不增加量比来源或生产权限，不等于完整W17完成。
- [P-02 冻结业务准备实施记录](implementation-auction-frozen-preparation-2026-09-08.md)：源码`eeb2ddc`、测试修复`74954fe`；一次横幅捕获、完整消息/逐票记录/通知集合提案已被实际dispatcher消费。修复后51项测试、静态检查及限定复审通过；没有补造量比来源，不代表完整W17迁移或上线完成。
- [P-02 量比与来源证据核对](auction-source-evidence-gaps-2026-09-08.md)：现行规则禁止跨批补量比，MarketStatistics同名字段尚无完整竞价合同；区分可先行的真实证据保留工程与需要产品/提供方确认的接源条件，不改变生产来源。
- [P-02 真实业务影子接线前置](p02-shadow-integration-handoff-2026-09-10.md)：按当前源码核对已完成的观察、冻结提案与影子内核；下一步须比较消息/逐票记录/通知集合，并让dispatcher消费同一旧提案。可信注册、完整业务adapter与八端口接线尚未交付，不重复开发来源保留、不把缺量比当VerifiedEmpty。
- [P-02 选集拒绝诊断实施记录](implementation-auction-selection-diagnostics-2026-09-10.md)：源码298ab0d；真实selector/tick保留空源、缺字段、有效行已通知的结构化事实，成功选票/文案/写库/通知集合规则不变。10项定向测试及静态检查通过，独立Spec/Quality Approved，本Task完成；保留库警告Minor与早期RED捕获局限。[计划](../superpowers/plans/2026-09-10-auction-selection-diagnostics.md)仅补必要输入事实，不补量比、不签发领域完成或生产权限。
- [W17完整业务提案比较实施记录](implementation-shadow-business-proposals-2026-09-10.md)：初版b142c9b、最终修复9b2a5df；组合绑定/提案存在性错误先运行期RED再修复，最终26项定向测试及静态检查通过，限定复审两项已处理且无新增问题，[本计划](../superpowers/plans/2026-09-10-shadow-business-proposals.md)完成。单Cargo历史违规、GREEN会话映射未知及测试前SHA遗漏显式保留，不称历史过程全面合规。同次比较实际提案并保留原旧输出，不授予发送或认证；真实P-02注册、adapter及效果纳管仍待。
- [W19 全量库存指标实施记录](implementation-w19-inventory-results-2026-09-08.md)：源码8094ffc、修复c10ec78；覆盖指定namespace全部业务日/状态、真实来源SLA、错误分母与扫描预算。N02缺lock冷读与Completed+Conflict漏等待年龄两个反例先RED再修复，最终107项测试/静态检查/限定复审通过。readiness/晋级消费者、完整留存与安全审计仍待，不将库存指标当生产健康许可。
- [W19 持久化最终化延迟检查结果](implementation-w19-results-2026-09-08.md)：初版27967be、修复6f8713c；通过Generic/P01/N02实际持久化reader与同事务业务全链读取，使用原始接受时间计算两周期目标/五分钟硬上限，保留人工、未决和冲突状态。审查发现的历史终态/资格遗漏先实际复现13种矛盾再修复，最终78项测试通过、本批静态零诊断，限定复审全部关闭；仅N02明确支持的局部occurrence约定，不代表生产注册。完整W19的指标汇总、留存与安全审计及生产消费仍待。
- [W17 影子执行内核结果](implementation-w17-results-2026-09-08.md)：源码`89128f3`，一次采集共用context/facts，精确比较真实决策/语义/渲染字节/完成提案；八类拒绝端口先计数再拒绝。17项新测试和52项相邻测试全部通过，最终静态检查本批零诊断，独立Spec/Quality通过。生产业务适配、全局端口纳管及W16激活证据仍待，不代表完整W17或迁移完成。
- [W15 实施结果](implementation-w15-results-2026-09-07.md) §10–13：依赖候选合同、实际 occurrence 读取、采集事务适配器及后续v3完整Unit集合已经存在；[§13 最新只读核对](implementation-w15-results-2026-09-07.md#13-v3集合与实际查询入口的最新边界-2026-09-10)纠正“仍只有单manifest”的旧状态。完整来源认证、同一认证快照的query/probe及调度联结尚未完成。
- [W16 实施结果](implementation-w16-results-2026-09-08.md)：已有跨进程broker接入真实initial写入、Generic发送与只恢复；最新T4D将精确业务恢复/finalizer写入接入同一worker许可（源码`4c07aaa`、测试修正`667ee4a`）。修正后受影响合批104项通过，含4个真实进程父测试，目标Clippy零诊断，限定复核全部关闭；未变邻域保留此前498项通过证据，不冒称修正后全量重跑。生产身份/受保护根、完整四actor共同fence、实际切换、具体Unit cursor及全Unit消费仍按[实施计划](../superpowers/plans/2026-09-08-push-foundation-w16-activation.md)交付，不能按基础切片标W16完成。
- [P-01/N-02 专用业务恢复接线结果](implementation-dedicated-business-recovery-2026-09-11.md)：源码3fed7aa，本Task完成；两类真实终态进入同一worker恢复/最终化，N02绑定实际持锁读取的年份文件。修后10项专用反例/完整编码通过，另37项未变范围回归保留通过证据（含真实进程），最终Clippy无新增诊断；独立Spec/Quality通过，3项补核完成、2项Minor保留。不是单次47项全绿，也不代表完整W16或生产迁移完成。
- [W16 合同裁决](activation-contract-decisions-2026-09-08.md)：澄清 shadow actor 与旧推送关系，固定外部批准包、同事务写入和paused确认顺序；真实监督器及恢复接线仍需实现证明，不是生产批准。
- [全 Unit 部署集合合同](activation-deployment-set-contract-2026-09-08.md)：全52Unit候选集合构造、真实读取、重读漂移和范围投影已实现至`e0cdd0d`，11项专项及最终golden1项通过，限定复核关闭排序缺陷。集合版持久消费后续进展见下方计划入口及[实施结果](implementation-w16-results-2026-09-08.md)，实际来源认证仍未交付。
- [P-02 同批数据修复结果](implementation-auction-volume-results-2026-09-08.md)：源码`0dda4cc`将选票、消息、入池和通知set绑定到一次采集的选中快照，同时保留持仓检测使用的完整原始列表。13项相关测试通过，最终静态检查在本次两份改动文件零诊断，独立规格/质量审查通过，无待修问题；[实施计划](../superpowers/plans/2026-09-08-auction-volume-batch-binding.md)的完整Unit迁移边界保持不变，生产monitor不替换。
- [集合快照与恢复存储接线计划](../superpowers/plans/2026-09-08-readiness-deployment-set-v3.md)：源码`4a3ef37`已将全Unit集合接入snapshot/material v3、stream v2及既有恢复/store链，补齐Core恢复责任，保留旧v2及冻结event/schema。首轮60通过/2失败，修正临时路径后新模块5/5通过，旧57项证据保留，最终Clippy目标零诊断；独立规格/质量审查通过，保留版本冲突直接测试及既有warning两项Minor，不代表实际source认证、query/probe或完整T6完成。

以下第三批状态是 **2026-09-06 的历史交接快照**，其中“W01--W21 运行时未实现”等描述不代表上述后续实现进度；原始输入/权限边界仍有效。

## 第三批历史交接

第三批规格完成：RFC/SQL/WBS 规格与机器合同已通过最终独立审查，原审查 findings 全部关闭；这不是推送系统改造完成。[第三批交接结果](implementation-batch-3-results-2026-09-06.md) 记录 FINAL SPEC PASS（0/0/0）、wave3 FINAL SCOPED QUALITY PASS（0/0/0）的精确范围，以及 clean `e35800a` 的 418 runs / 5023 assertions 和临时 SQLite 证据。最终状态提交未重跑五套测试，规格完成不提升 `PROVISIONAL` 发布状态。

## 事实边界与阅读顺序

1. [已批准决策](grill-decisions-2026-09-02.md) 与 [设计来源目录](../../design-source-catalog.v1.json) 记录设计约束及来源裁决。
2. [65-kind 能力目录](push-capability-catalog.md)、[机器目录](push-capability-catalog.v1.json) 和 [源码证据 manifest](push-evidence-manifest.v1.json) 记录隔离分支在 Rust 基线 `07781bf386aafdf202851ae928efee8920387058` 的源码接线。
3. [RFC 输入 manifest](rfc-input-manifest.v1.json) 冻结下列八份原工作区输入的原始字节、尺寸、SHA-256、角色与冲突。冻结不提升其事实权限。
4. [实施 RFC](push-system-implementation-rfc.md) 定义身份、应用结果、完成权威、跨库恢复、调度/readiness、shadow/activation/operator、保留期与验收合同；[独立 SQLite CLI 规格](push-system-foundation.v1.sql) 与 RFC 嵌入逐字节一致，仅在临时数据库验证。
5. [WBS 机器事实源](push-system-wbs.v1.json) 定义重建的 W01--W21、52 Unit 估算/依赖/门禁，RFC 内只保留生成摘要。828.99h 基线、994.79h 缓冲后工时及条件日历场景是可复算估算，不是承诺日期。[第三批实施计划](implementation-batch-3-rfc-wbs-2026-09-06.md) 和交接结果记录最终独立审查结论与本批明确不交付的边界。

严格 RFC 门禁仍返回 `rfc_status_provisional`、`wbs_status_provisional`、`rfc_html_missing`、`ci_rfc_gate_missing`；显式 `check-catalog.rb --root . --check` 仍返回 catalog/manifest 两项 provisional。W01--W21 运行时未实现，52 Unit 未迁移、未 shadow/live promote；Rust、生产 DB schema、配置、模板及业务行为未改。蓝图 §24/§25 去拟议化、通用离线 HTML builder、RFC HTML 与统一 checker/CI 接线未交付。未部署，无真实接收、用户已读或交易收益证明；PaperBuy/Watchdog 仍为原混乱工作树排除项，160 个冲突未解决。

## 八份不可变输入

| 输入 | 角色与限制 |
| --- | --- |
| [架构蓝图 Markdown](../Project_Architecture_Blueprint.md) | 当前架构与拟议设计的语义输入快照，原文的 CURRENT 不直接证明本隔离分支或部署状态。 |
| [架构蓝图 HTML](../Project_Architecture_Blueprint.html) | `derived visual snapshot`，不是独立语义真相，也不是本批已重建的 HTML。 |
| [历史再分析](comprehensive-reanalysis-2026-09-05.md) | 2026-08-31 至 09-04 生产证据分析、风险与验收样本。 |
| [聚合证据 JSON](recent-push-evidence-2026-09-05.json) | 脱敏历史快照，不等于实时生产状态或跨库原子快照。 |
| [原 67-kind 视图](all-push-kinds-2026-09-05.md) | `non-baseline evidence`，不覆盖本分支 65-kind 能力目录。 |
| [原文档硬化计划](push-documentation-hardening-plan.md) | 验收要求及后续 HTML/CI 边界输入，不是当前执行完成记录。 |
| [再分析程序快照](reanalyse-recent-pushes.rb) | 派生程序快照，本任务不执行生产读取。 |
| [内部一致性程序快照](verify-reanalysis.rb) | 证据内部一致性校验快照，本任务不执行。 |

67 与 65 的差异不通过补造能力消除：`PaperBuy` / `Watchdog` 在原混合工作树存在，本隔离分支未移入，仅保留为 `excluded_worktree_additions`；不得据此创建本分支 MigrationUnit 或激活证明。原快照中的状态、规模和历史行号不是当前源码证据身份。

## 离线输入门禁

在仓库根执行：

```bash
ruby scripts/architecture-docs/check-rfc-inputs.rb --root .
ruby scripts/architecture-docs/test/rfc_inputs_test.rb
```

门禁仅校验 manifest 的契约、相对路径、普通文件身份、原始字节尺寸与 SHA-256，不写入或修复输入；读取前检查真实路径包含关系，拒绝目录及任何输入路径链接。成功返回 `0`，内容失败返回 `1`，参数失败返回 `2`。`PROVISIONAL` 在此仅是输入 manifest 的固定状态，校验成功不是发布门禁通过。

不得对八份输入做换行规范化、格式化或重生成。两份导入 Ruby 只执行 `ruby -c` 语法检查；不要把本入口的导入/校验步骤变成生产查询或脚本重跑。
