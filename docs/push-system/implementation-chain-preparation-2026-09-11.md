# 盘后产业链固定准备：实施进度与验证边界

最新开发状态（2026-09-14北京时间）：已修复龙虎榜历史结果时间校验缺口。70992真实反例后，80508同四项、80583原范围166项回归及87868消费者Clippy均通过；限定独立Spec/Quality通过。源码/日志摘要已核，保留56条测试告警和233条消费者告警，不冒称全仓无告警或完整Task2通过。完整历史身份/Status/重试/提交矩阵、Task2–4/W15–W21/52Unit仍未完成。首批手动包不等待本片，运行根方案尚待确认，本轮未启动或替换monitor。详见[龙虎榜恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)。

前次板块Retry续接验证（2026-09-12）：session50677于11:47:38Z退出0，105项全部通过；编译5m00s/测试65.22s、49告警，585项源码前后/当前与日志SHA一致。真实Retry确认后取消/重开，完整退避后以原request/次数/策略续接，原首组事实不改、跨代引用保持、最终总3RPC与两条原字段BR159均通过，关闭68550反例。6个正常文件修改，原测试/Cargo/固定SQL/codec形状保持。消费者Clippy session64389于11:52:07Z退出0，3m58s、lib202/monitor2告警，585项及日志核一致，较17868无新增/移除诊断种类；仅编译未运行monitor。该冻结点尚未覆盖随后新增的实际授权失败用例，当前结果以上条为准。完整授权/故障/兼容矩阵、错误终结材料、Task2/真实调度和52Unit仍未完成。

前次成功Response提交恢复验证（2026-09-12）：session25581于10:56:05Z退出0，104项全部通过，编译5m56s/测试72.74s、49条告警；585项before/after/当次当前与日志SHA独立一致。真实Concept最终COMMIT争用后回滚，关闭重开换代后只用原Response补最终事务；总RPC仍3、原begin/result及Industry旧审计不改，两条最终BR159的完整原字段/时间/批次均保持。消费者Clippy session17868已于11:01:11Z退出0，编译3m54s，lib202/monitor2告警；585项源码前后/当前与日志SHA一致。仅编译未运行二进制，新增三类诊断已记实施台账，非零告警验收。ConfirmedRetry、错误终结及其他故障/兼容矩阵、完整Task2/真实调度/全部52Unit仍未完成。此前33118新故障例的唯一RED已关闭，测试/固定SQL未为修复放宽；以下历史证据仅代表各次冻结版本。

## 本轮实际修复：检索词生成的停止保护

2026-09-13。真实反例81997证明：模型返回带底层原因的`PreparationStop`后，旧`if let Ok`仍继续三个搜索和后续深度、简化、总览分析，最终返回成功。现仅修改[准备流程](../../src/pipeline/chain_analysis/preparation.rs)：强制停止错误原样返回并由原阶段封装保留前序事实；普通模型失败仍按原默认检索词降级。查询构造、每次4条/15秒、10条截断及成功报告行为不改。

同一反例85521于03:36:43Z通过；组合82792于03:41:55Z实际156项全通过，原154项保留，新增检索词停止及v8真实迁移COMMIT回滚两个用例。601项输入前后及日志摘要一致。限定独立检查未发现本增量待改问题，但它不是完整Task2审查；该冻结点消费者52147于03:47:34Z编译通过，215条告警与上次数量/类别相同，未运行二进制。后续总览修复与验证见下；真实搜索内部停止传播、超时与失败阶段内部观察持久化仍未全部完成。

## 本轮实际修复：总览模型的停止保护

Overview反例49775实际返回成功，证明原总览helper将必须停止的错误降级为缺失总览。现[总览helper](../../src/pipeline/chain_analysis/mod.rs)保留原错误与cause上抛，[唯一准备调用](../../src/pipeline/chain_analysis/preparation.rs)接收并在组装成功报告前停止。普通模型失败、空白响应仍可降级，非空响应保留原字节，prompt和成功内容不改。

1659于04:09:02Z同一精确用例通过。初次格式检查失败已如实记录，终态后仅展开guard；92844于04:21:31Z在最终格式源码上完成同例及两个旧调用方共3项回归。1577于04:26:16Z完成157项原完整过滤组，原156名称全部保留；601输入与日志摘要独立核，编译1.65秒、测试135.46秒、51条既有测试告警。限定独立检查未发现行为问题，唯一格式项已收尾；不将局部独立检查代替完整Task2审查。后续42516消费者Clippy已通过，告警及当前范围以上述最新状态为准；未运行任何生产二进制。

## 龙虎榜续接：逐次RPC接口与完整持久验收分开

已核定[龙虎榜真实效果与恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)。原一次查询最多四次客户端RPC，终态后才有一条R-04采集审计；不能整批返回后记map就称可恢复。真实loopback首例和旧公开查询共用逐次接口已实现，67604首例与74652的162项回归通过，95400同源消费者Clippy退出0，新增10条告警如上方状态，限定独立审查保留Low维护提示。自然日时钟、同库journal/审计、原批次/净额投影、真正SQLite重开仍按完整Task2继续。当前新持久prepare的DragonTiger未迁移停止未移除，不提前接管monitor。

## 簇搜索与盘后搜索：必须停止错误已获限定验收

原[observed_search](../../src/pipeline/chain_analysis/preparation.rs)将搜索错误统一降级为空背景。32687于07:10:22Z实际退出101：[公开prepare反例](../../src/pipeline/chain_analysis/preparation_search_typed_stop_tests.rs)的簇新闻、盘后催化两场景均返回Ok，且继续全部5条搜索和4类模型，证明必须停止错误被吞；不是编译或fixture失败。

修复只在preparation.rs共享搜索分类点保留原PreparationStop/cause，并由簇搜索、盘后搜索及render逐层传播Result；原测试不放宽，普通错误、空来源、未配置和15/8秒期限不变。78677于07:19:17Z退出0，16项限定准备回归通过，包含该公开测试的两个实际场景及原15项；现已执行原错误链、前序业务事实与精确停止位置断言。三文件独立Spec/Quality均通过、0个发现。8691于07:23:38Z完成同源lib/stock_analysis/monitor Clippy，退出0、223条告警与前轮一致；仅编译未运行二进制。

主控核607项源在16项测试、Clippy的前后与当前完全一致；16项日志SHA为a928e0b61b6b61ae34294f195482c194aa049cfc6308f72c6cde34ff3593bd7f，Clippy日志SHA为1d2353e8b9083f646f7d83a788430d856de5af33b90934a43f2a9996ad110f4b。之前160项是v9修复冻结点，不与16相加、不称当前全量重跑。真实超时取消/未决保护、失败阶段内部观察持久化及后续source/model日记仍待；此修复不是完整Task2或生产验收。

## 持仓第二批概念 v9：真实接线和缓存历史修复已获局部验收

已落地的 [v9 持仓概念恢复](../../src/push_foundation/intent_store/chain_post_close_position_concept_rpc.rs)采用七张独立事实表，复用[同一六并发驱动](../../src/push_foundation/intent_store/chain_post_close_concept_rpc_driver.rs)和原协议编码；以首次保存的持仓缓存材料及完整持仓下标标识请求，不按代码去重，不借用首批结果或缓存进度。实际 v8→v9 迁移登记完整135对象、只新增28对象；旧107对象及旧登记保持。

最初5797仅取得缺少接口的编译失败，未执行业务；正常接线后85310完成成功/真重开首例。随后52810暴露重试耗尽被错误拦截，修正未发布v9的终态校验及原状态只读投影后，49129同一测试通过：四次真实请求、原退避策略、完整BR159、业务错误终态及重开零重放均有断言。旧v1–v8 SQL和codec wire不变。[两项真实流程测试](../../src/push_foundation/intent_store/chain_post_close_position_concept_rpc_tests.rs)已包含在41215的159项限定回归中，原157测试名称全部保留；42516同源消费者Clippy退出0，lib221/monitor2条告警，不是零告警验收。

独立审查发现的I1历史缺口现已修复：[缓存历史屏障](../../src/push_foundation/intent_store/chain_post_close_position_concept_rpc.rs)逐条核全部final早于cache的版本、时间及代次关系，保留合法无cache部分进度和按原终结顺序应用的成功前缀。[公开反例](../../src/push_foundation/intent_store/chain_post_close_position_concept_order_tests.rs)先真实完成两条同码、不同完整下标的缺失请求，再只改五个历史字段且保持外键、目录、审计及业务源完整。原68754在精确拒绝期望失败；正常修后11829在更早准入返回SchemaRejected，被测试unwrap误判。只将两处测试改为准入→读取串接后，45108于06:52:55Z实际160项全通过，原159名称全部保留；真关闭重开、两次精确拒绝和零重放/回写断言均执行。原审查者限定复审I1/M1均已处理，无新增问题；完整v9故障矩阵仍未全部验收。

当前适配器仍停在龙虎榜阶段，不是完整盘后恢复或生产接管。合法部分进度、并发/取消/提交故障和迁移损坏的完整矩阵、后续来源/模型/报告/逐渠道发送及Task3–4均仍在范围内。

## 概念真实 RPC v7：局部验证通过，完整恢复仍待

2026-09-13。新增 [v7 SQL](../../src/push_foundation/intent_store/chain_post_close.v7.sql) 已按核定原字节纳入隔离工作区，SHA256为 `3e5c0c8443c5fcc6ea3c99bd318134d86579aba2d16bc055e51036ad639509ca`。除初期内存SQLite加载检查外，现已取得实际Rust v6→v7及真重开证据：旧BusinessError、ReturnedPendingCache、ReturnedApplied三类状态保留原事实且不重发已执行的概念请求；原请求begin代码关联损坏的迁移拒绝反例也已转绿。LegacyUnconfirmed、其余合法前缀与完整损坏矩阵尚未全部验证，不能把三类通过表述为整个旧库迁移完成。

- 已写的共享客户端：[BoardQuerySession](../../src/grpc_client/board_attempt.rs) 用封闭 Directory/Memberships 身份复用实际授权、路由、重试和结果捕获；[ConnectedBoardQueries](../../src/data_gateway/grpc_source.rs) 从显式source预连接后提供两类查询。单用户范围没有取消来源认证，也没有把不支持的ExternalV1成员查询放行。
- 已抽取并复用的旧业务：[MembershipRequest](../../src/data_gateway/board_runtime.rs) 保持代码验证和原请求hash；[工具JSON投影](../../src/agent/tools_sector.rs) 保留完整原字段、顺序和VerifiedEmpty错误。配置仍在概念RPC之前显式固定，不隐藏阈值或读取环境。
- 已实现并有局部实测的恢复路径：真实occurrence/attempt先记录；返回后保存raw与Status材料；已确认终态在同一事务生成旧begin/result兼容投影、完整BR159和v7 final，再复用原cache/cluster。成功重开、结果提交失败不再请求和历史审计损坏拒绝已有验证，完整重试/并发/协议错误矩阵仍待。跨运行恢复不能伪造旧owner；旧begin不再冒充新RPC最早发生时间。
- 已通过的首例：[实际prepare提交故障测试](../../src/push_foundation/intent_store/chain_post_close_concept_rpc_tests.rs) 经29370精确运行及62871组合运行，实际证明BoardConstituents只调用一次、结果COMMIT失败保留真实commit原因，同adapter重入及真重开均不再请求，原配置/未决begin和head边界保持。此前73994、88769的编译RED保留，不改写为当时已执行业务断言。
- 首例合同维护：同adapter重入明确为 `ResultUnconfirmed`，而非初稿的 `AuthorityRejected`；依据是[既有批次提交失败回归](../../src/push_foundation/intent_store/chain_post_close_concept_batch_tests.rs)及先检查取消标志的实际入口。只修新测试一行，首次commit原因、重入不新增阶段观察和真重开零请求等断言保持；反向SHA还原已核，后续两次运行均通过。
- 旧版本维护：[原迁移测试](../../src/push_foundation/intent_store/chain_post_close_v3_migration_tests.rs) 保留伪7库、将已知损坏归为SchemaRejected；新增未知8在重开前后Unsupported且不修复。原精确v3 reader仍拒绝7，其他旧业务断言不放宽；62871中该完整测试已通过，不等于v6→v7业务事实迁移矩阵通过。
- 已关闭的一项真实缺陷：[selection父内容反例](../../src/push_foundation/intent_store/chain_post_close_concept_rpc_tests.rs) 原先在恢复读取和实际迁移中均接受目录外板块代码，24652于21:24:15Z失败；两文件修复后原命令/断言在54571于21:47:29Z通过，拒绝且旧全表/catalog保持、无新封存残留。56682中原143项、四旧alias和新纯规则、selection均通过；唯一新增Unknown例失败于迁移前错误空缓存oracle，已纠正测试并另行复跑。共享原匹配规则和Available原final内容校验已落盘，不以这些局部通过证明完整目录/selection故障矩阵完成。

固定schema、共享client或首例通过都不代表Task2完成；持仓及第二批概念、其余来源/模型、报告与逐目标发送、真实调度和强完成状态仍按完整计划交付。

当前编译告警：75419相对86972新增6类，涉及board布局的OR范围写法、新concept模块的显式Err再unwrap与多余引用、RPC事件枚举大小差异及client/session两个八参数接口；没有移除旧诊断类别。lib告警202→210、monitor仍2。这是非零告警的编译通过，不是质量终审或生产运行证明。

## 后续持仓接线：已核实的来源边界

2026-09-12只读核对，尚未实施。本轮板块COMMIT恢复修复不包含下列改动：

- 当前[真实准备适配器](../../src/pipeline/chain_analysis/preparation.rs)的Positions读取[stock_position中的open行](../../src/database/positions.rs)，按buy_date降序，不是UserPositionSnapshot接口；不能仅凭“持仓”名称替换业务来源。返回前降维为code/name/return_rate，恢复还需固定首次完整数据库行、实际顺序和本地查询时刻，不能用业务日或updated_at伪造源快照时间。
- 非空持仓之后还会再次读取概念缓存/查询缺失概念。当前本地概念journal绑定原涨停代码序列，不能直接复用为第二批；第二次缓存读取也发生在首轮可能写缓存之后。需要明确的持仓概念阶段身份及其首次缓存材料，保持原代码顺序和可能的重复持仓，不靠调用次数猜阶段。
- [概念工具](../../src/agent/tools_sector.rs)调用[板块归属gateway](../../src/data_gateway/board_runtime.rs)，内部还有实际gRPC重试和默认审计。现有外层raw provider测试证明的是工具调用/结果与缓存恢复，不能据此推断内部每次RPC及自提交审计都已迁移到同库journal。该接线仍属于完整Task2，既适用于后续持仓概念，也须核对首轮概念的真实生产适配器；当前没有新增真实来源调用或生产验证。

### 2026-09-13 持仓恢复实施合同（首例已通过，完整故障矩阵待验收）

在现有最高v7之后新增受控v8，不改旧v1–v7 SQL/codec或原持仓来源。计划中的两个专用材料表分别保存首次完整持仓与持仓阶段独立的概念缓存；它们与既有run/head/lease在同一业务库短事务内提交，不新增旁路库。

- 完整持仓保存id、code、name、buy_date、buy_price、quantity、status、sell_date、sell_price、return_rate、created_at、updated_at、chain_name、st_type；原时间文本、NULL、浮点值及首次行序保留，再投影旧三字段。仍是open全部行/buy_date降序，不新增同日排序或按代码合并规则。
- 第二概念阶段使用明确的`position_concepts`接点，旧默认实现委托原`concepts`；新本地实现绑定持仓原码序和独立缓存材料，不靠调用次数判断。缓存在该阶段按原本地七天规则重新读取完整有效行，保留非请求代码，不能复用启动缓存；重开后只用保存材料。
- 原日期文本解析和坏缓存整批失败规则必须保持。存储完整原值不代表来源已认证，不把查询时间、行updated_at或业务日冒充持仓确认时间。
- 首个贯通用例固定成员、别名、无关三条open持仓及一条应排除的closed行；在真实v7完成概念/聚类/目录之后才加入两条持仓专有缓存，再实际迁移、读取，并验证完整五键概念图和原纯诊断。
- 修改持仓和缓存来源后真正关库重开，仍应返回首次三条持仓、五键概念和原诊断，不修回来源、不重写已确认事实、不再发起原目录/RPC。现成目录测试服务包含首次Industry重试，健康目录实际为3次RPC；重开应新增0次，不能把两个板块种类当成两次请求。
- 第一例全缓存命中后明确停止于尚未迁移的DragonTiger；缺失概念暂时显式停止，后续仍须完整接通六并发/RPC/审计/缓存应用、旧库迁移故障和重复码等矩阵。该中间停止不是最终业务交付，更不是Task2或52个Unit完成。

实现前兼容核对已完成：锁定Diesel 2.3.7对Timestamp先取SQLite文本视图，Julian数值回退也解析该文本。因此created_at/updated_at保留TEXT原字面，并接受旧读取器已支持的数值时间戳文本投影，不声称备份物理存储类型。新reader与恢复校验已落候选实现，尚待实际验证。

首例测试与验证：[持仓重开贯通用例](../../src/push_foundation/intent_store/chain_post_close_positions_tests.rs)现934行，父module两行及原1812行父测试保持已核。每次接管按旧合同独立推进head：首次接管和两材料合计+3，重开接管+1而材料恢复+0。首例保存完整14字段、三条open行原序与完整五键第二缓存；修改真实测试来源并真关库重开后仍返回原材料/纯诊断，原事实、BR159、Foundation/scope与既有目录保持，新增RPC为0。仅覆盖第二cache全命中，随后明确停于尚未迁移的DragonTiger。

保留失败过程：18080因缺观察时钟接口编译失败，57780两处offset比较产生四条i32/i64编译诊断；修后44221编译成功但在实际迁移返回SchemaRejected。根因是v8 registry先于seal形成暂时deferred外键缺口，全事实检查提前拒绝；只将原layout-8全量校验移到新DDL后、registry前，仍保留旧先验、seal前事实校验、最终verify及同事务COMMIT，旧SQL/codec/测试不改。76752于2026-09-13T01:01:56Z退出0（1通过，6.60秒测试），原完整恢复断言实际执行；不是只取得迁移成功。

同源Simple修后验证23054于01:02:39Z退出0（2通过，含版本边界）：必须停止时保留原cause及先前事实，模型序列止于Deep/Simple、不调用Overview；普通模型错误仍降级。62555于01:52:29Z完成154项组合回归（2.37秒编译、131.58秒测试、51条告警），原152项及过滤命令保持，新加恰上述两例；600项before/after/当次当前及日志SHA独立一致。消费者16084于01:57:01Z退出0，3分23秒，仅编译lib/stock_analysis/monitor、不运行二进制；600项与154回归同源且日志SHA核。lib213、monitor2告警，相对13193增加3条范围模式写法告警，新增标题+源码路径类别为cluster与chain facade两类，无移除；保留非零告警，不称质量终审。完整v8故障矩阵、第二批missing RPC与其他Task2效果恢复仍未验收。

## 历史实施记录

下列记录保留各次实现和验证过程；当前板块结果以文首为准，旧失败及当时的待办不覆盖后续证据。

当前板块片（2026-09-12）：[真实请求贯通测试](../../src/push_foundation/intent_store/chain_post_close_board_tests.rs)及[有界本机gRPC fixture](../../src/grpc_client/board_loopback_fixture.rs)首轮 session8214（08:08:26Z–08:12:47Z）编译退出101，581项源码与日志摘要一致；6处正常接口缺失、1处测试Diesel trait导入问题，后者只修导入、不改断言。v5 SQL已落盘且与核定草稿逐字一致，五版SQL在内存SQLite加载成功，共63个有定义对象；这不是Rust迁移或贯通测试通过证据。迁移/gRPC实际请求/同库恢复正在接线，静态发现的授权metadata丢失及重复请求路径问题已反馈实施者，尚无修后运行结果。新增[板块验收矩阵](board-directory-recovery-validation-2026-09-12.md)固定成功、协议错误、SQL/COMMIT、未决/重开及兼容验证要求。下面90项与Clippy仍只属于此前冻结版本，完整Task2未完成。

当前新增片（2026-09-12）：[BR159 同事务追加测试](../../src/database/data_acquisition_audit_transaction_tests.rs)先在 session62331 取得两个缺接口编译 RED，补[原审计模块](../../src/database/data_acquisition_audit.rs)后 session21419 精确首例通过。随后 session76721（07:26:35Z–07:34:22Z）合批90项全部通过：审计19+原盘后71，3379过滤，6分34秒编译/55.63秒测试，47条告警；579项 before/after/当次当前与日志 SHA 一致。双驱动交替追加与独立能力/来源前态、原完整链reader、坏输入/坏尾/原guard保持、audit/chain SQL abort后外层回滚、真实COMMIT DatabaseBusy后旧链保持/候选不可读/正常重试、缺表与query_only不修复均通过。接口不管理连接/PRAGMA/事务，receipt提交前仅是候选，失败由外层回滚；原字段/hash/DDL与旧测试预期保持。消费者Clippy1348已于07:41:10Z退出0（lib199/monitor2告警），579项源码及日志摘要一致，较31234无新增或移除诊断种类；仅编译、未运行monitor；开库的schema/全链准入、板块实际RPC与后续模型/报告发送、完整Task2仍待。下文为历史冻结点，不把90项算成52个Unit迁移完成。

日期：2026-09-11。状态：**Task 1 工程验收通过；未生产切换**。实施基线42ce098，初版60e18ae，最终源码40b0a63；初审规格符合，唯一Important经原作者修复，限定复审确认已关闭且无新增Critical/Important。本记录不冒充生产批准或部署证明。[完整实施计划](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)的Task 2–4及全项目W15–W21、52个迁移单元范围不变。

最新Task2局部证据（2026-09-12）：session85939（06:35:05Z–06:43:43Z）71项全部通过、3392过滤，编译7分07秒、测试70.25秒，47条告警；578项源码before/after/当次当前及日志SHA均独立一致。[候选维护](../../src/pipeline/chain_analysis/preparation_tests.rs)通过真实prepare验证目录/候选的原PreparationStop与测试私有cause保持、零后续效果，普通错误仍Unavailable并完成；[v4迁移维护](../../src/push_foundation/intent_store/chain_post_close_v4_migration_tests.rs)从真实v2已确认/未决运行产生v3缓存应用，实际迁移v4后旧事实/登记/全库版本保持，重开零provider。真实COMMIT故障回滚新增12对象及v4登记、保留旧31对象和原事实，释放读锁重开仍v3且可重试迁移；异名附着拒绝前后目录/事实不变。首次session75226仅一个测试私有模块导入E0603、未执行业务测试；只改测试cause后本轮通过，不包装成生产修复。同版本消费者Clippy31234退出0，578项源码与日志摘要一致；库199/monitor2告警，较75032新增聚类恢复导出未使用和聚类返回类型复杂度两类诊断，未宣称零新增。完整Task2、Task3–4及52个Unit尚未验收。

上一聚类修复session43450同67项全部通过，三个原图/父子事实关系自然RED已关闭，原测试/SQL/codec未改；此前失败后未执行的保持/重开断言均在该轮通过。材料写前精确往返与父子两边细分的直接证据仍须核实，不把新迁移维护或一次有限数值案例当全输入证明。后文为分阶段历史记录。

## 已写入实际业务路径的改动

| 行为 | 当前代码接点 | 已有证据与限制 |
| --- | --- | --- |
| 旧入口只执行一次准备，再返回原报告 | [mod.rs](../../src/pipeline/chain_analysis/mod.rs)的run_chain_analysis；[preparation.rs](../../src/pipeline/chain_analysis/preparation.rs)的prepare_chain_analysis | 新旧空池及显式外部adapter非空流程通过；不是生产部署证明 |
| 保留实际输入和采用的事实 | preparation.rs的PreparedChainAnalysis、prepare_chain_analysis_with_io | 固定业务日、涨停输入、概念聚类、生命周期、持仓匹配、龙虎榜和宏观输入已进入只读结果；缓存和chain_daily仍会由生产adapter写入，不是纯函数或持久检查点 |
| 留存本地模型调用和搜索材料 | preparation.rs的IoModel、observed_search、render_with_io | 本机真实HTTP协议覆盖检索词生成、深度、简化、总览；保留prompt/system/mode和分析器原返回文本，逐次保留搜索结果。没有证明远端请求正文、实际物理模型版本或真实提供方身份 |
| 模型原文与报告格式分开 | preparation.rs的ModelCall；mod.rs的build_report | 模型返回的空白、Unicode原样保留；报告继续原有去结论/评分行及清洗规则。旧成功/失败HTTP协议和纯报告回归通过 |
| 保留龙虎榜原日期政策 | [fetchers.rs](../../src/pipeline/chain_analysis/fetchers.rs)的fetch_lhb_observed | 源码保持按实际请求时Local自然日取数，单独保留请求日期/本地时间及Gateway证据；不改成pipeline业务日。此项未调用真实龙虎榜来源验收 |
| 中途失败保留已取得的事实 | preparation.rs的PreparationFailure、observe_stage | 已写入固定输入、先前事实和失败阶段的只读观察；验证状态见下表。返回错误不证明先前写入回滚，也不是可自动重试许可 |
| 完整保留候选和目录来源证据 | preparation.rs的prepare_candidates | 保留目录原批次顺序、实际映射与选用板块代码，在原降级投影前复制候选五项批次证据。成功空记录仍按旧规则Unavailable，但不再丢失来源时间/观察时间 |
| 首次结果提供版本化封存与无外部效果解码 | preparation.rs的to_artifact_bytes、from_artifact_bytes | 私有版本1包络保留完整输入、事实、模型/搜索材料及原报告，记录实际聚类阈值和缺失来源身份；拒绝坏版本/截断/遗漏字段/重复键/非有限数值。只接受自身确定性原字节；不是Foundation身份认证、已落库或重启恢复 |
| 旧回归进入同一实际准备入口 | mod.rs的resolved_chain_facts_persist_match_and_render_without_external_sources；旧失败协议回归 | SQLite单例由新入口实际调用原DAO/streak；失败协议仍执行3次本机HTTP。移除旧renderer、旧聚类写入包装及旧成功协议测试，后者由新入口4次实际HTTP覆盖取代；原纯搜索辅助测试单列保留 |
| 生产和旧持仓回归共用匹配判断 | mod.rs的match_position_diags；preparation.rs及旧diagnose_positions调用点 | 删除两个调用方中的重复判断；公开准备验收六种情况，保留原先簇匹配顺序、别名、收益率及孤立股不算簇成员的语义。旧真实持仓/缓存SQLite测试未删改 |

## 实际验证

所有命令均由主控单队列运行；每次Rust源码冻结，记录执行前后摘要及原始日志。测试不运行monitor、不读取真实.env或业务库，不调用真实模型、行情或通知渠道。

| 验证 | 实际结果 | 能证明的范围 |
| --- | --- | --- |
| 新旧空池及非空无模型准备 | session93513：3通过、0失败、3395过滤，源码未变 | 固定日期/原报告与显式外部效果序列；是早于后续观察扩展的切片证据 |
| 模型/搜索留存及旧协议、报告回归 | session92921：9通过、0失败、3390过滤；编译2分20秒、测试0.07秒，源码未变 | 本机合成模型协议、5次合成搜索、原响应、跨自然日催化观察与原报告政策 |
| 旧真实SQLite写入/生命周期回归 | session95144：1通过、0失败、3398过滤；编译1.61秒、测试0.17秒，源码未变 | 新测试进程、完整名称精确单例、独占0700临时目录，保留真实DAO/streak覆盖；不是新准备同事务恢复证明 |
| 核心失败事实保留及相关回归 | session26279取得缺接口编译RED；实现后session1035：7通过、0失败、3393过滤；编译2分48秒、测试0.01秒，源码未变 | 概念失败、写入后生命周期失败、持仓失败均保留此前实际事实且不继续模型；只证明受控失败行为，不是生产事故复现或持久恢复 |
| 候选完整证据及相关回归 | session74643取得缺接口编译RED；实现后session73322：8通过、0失败、3393过滤；编译3分16秒、测试0.01秒，源码未变 | 实际准备入口覆盖非空成功、VerifiedEmpty及异常成功空记录；保留完整目录/候选批次。此命令不含HTTP或SQLite，不计为持久恢复 |
| 版本化封存及相关回归 | session84637取得8个缺接口编译RED；实现后session97968：9通过、0失败、3393过滤；编译2分53秒、测试0.05秒，源码未变 | 真实准备流程配合合成外部模型验证原字节/Unicode、确定性映射、来源缺失、损坏拒绝及安全错误；本轮不含HTTP/SQLite。有限数值不能精确保留时允许明确失败，不允许静默损失 |
| 宏观/可选失败/数量上限及候选封存 | session84992：12通过、0失败、3393过滤；编译3分13秒、测试0.06秒，源码未变 | 宏观四种输入/回退情况、模型和搜索失败/普通空结果、8深度/12简化/20候选上限；原候选三情况均经编码解码后验证完整来源证据。是已有行为的补充覆盖，不人为制造RED |
| 初版非DB回归 | session41530：18通过、0失败、3386过滤；编译3分15秒、测试0.13秒，源码未变 | 新准备9项、旧空池/纯报告4项、迁移失败HTTP1项、原纯搜索辅助4项；仅本机合成协议，不代表生产投递 |
| 初版真实SQLite回归 | session15465：1通过、0失败、3403过滤；编译1.94秒、测试0.18秒，源码未变 | 原完整测试名精确单例、新进程与独占0700目录；新入口真实写库一次，返回原DAO生命周期及预期匹配/报告 |
| 初版格式和实际消费者编译 | 五改动Rust文件rustfmt检查通过；session37643定向lib/stock_analysis/monitor Clippy退出0，2分12秒，源码未变 | 只编译、不执行两个bin。188条库告警及2条monitor告警均未指向本次改动文件，不宣称全仓零告警 |
| 持仓六种语义提取前基线 | session31317：1通过、3404过滤；编译4分20秒、测试0.00秒，源码未变 | 新公开入口覆盖直接成员/概念/别名/无匹配/先簇概念胜过后簇成员/孤立股；已有行为维护性验收，不伪造RED |
| 修后非DB回归 | session2294：19通过、0失败、3386过滤；编译3分51秒、测试0.25秒，源码未变 | 初版18项加新持仓验收；同一源码覆盖真实准备、原字节、数量上限及本机HTTP成功/失败 |
| 修后两个真实SQLite单例 | session40314与41748分别1通过、3404过滤，测试各0.25秒，源码未变 | 旧持仓/新鲜缓存回归及公开prepare→原chain_daily DAO/streak回归，各用新进程、独占0700目录；不是一个两项共库命令，也不是生产数据库证明 |
| 修后格式和实际消费者编译 | 五改动Rust文件rustfmt检查通过；session90359定向Clippy退出0，2分29秒，源码未变 | lib和实际stock_analysis/monitor只编译不执行；188条库及2条monitor旧告警未指向本次文件 |

上述测试成功批次均仍有43条既有测试编译告警。初版三份证据仅对应60e18ae；修后19项、两个SQLite单例及消费者Clippy的四份前后源码摘要彼此一致并匹配修后源码，四份日志摘要均已复核。19+1+1来自三个测试命令，不写成单次21项。具体用例位于[preparation_tests.rs](../../src/pipeline/chain_analysis/preparation_tests.rs)、mod.rs的tests与[旧失败协议回归](../../src/gate_d_chain_analysis_regression.rs)。旧持仓缓存用真实时钟和七天TTL，仅在新库、有界正常时钟下证明命中，不声称任何跳钟/过期场景都硬禁provider。原纯搜索辅助测试的超时用例不等于新生产观察分支的超时实测；本轮没有全仓测试或生产调用证明。

## 仍须完成

- 最新范围按[单用户本地模式](single-user-local-scope-2026-09-11.md)：复杂可信身份/角色审批已从本次范围排除，不再等待身份发行方；本地受约束入口仍须实际接线，事务、任务锁和防重不取消。
- Task 1已完成本阶段工程验收；新宏观/逐簇/盘后超时观察的直接验收与既有告警两项Minor继续保留，需在后续故障验收/整分支审查中处理，不冒称已解决或已零告警。
- Task 2：同业务库中的实际来源/模型阶段检查点、原报告字节、chain_daily原子进度、报告保存、冻结目标及逐目标发送日志，包含重开和崩溃验证。2026-09-12安装基础7项测试全部通过，覆盖安装/重开、五种损坏拒绝、未来版本、连接/事务、封存写保护和真实提交争用回滚。本地run和首个概念阶段已有实现及下述局部验证，完整业务与发送恢复仍待。
- Task 3–4：真实应用/定时器/启动入口、窗口与业务日资格、本地执行配置与任务锁、必达策略、强完成游标和切换/回滚门禁。发送失败误封日尚未修复，不用局部准备成功来关闭这个问题。

外层采集边界已再次核对，尚未迁移：[实际应用入口](../../src/app/modes.rs)在prepare前先取涨停池，再取财联社20条快讯，只将股票投影与最多15个标题拼接文本传入分析。[名称分片采集](../../src/market_analyzer/limit_up.rs)及[新闻gateway](../../src/data_gateway/global_news.rs)已有外部请求与数据库审计效果，不能把整个外层当纯读或可任意重跑；[审计默认入口](../../src/data_gateway/review.rs)使用全局业务库。新闻请求只有provider/limit，没有业务日参数；源码“今日”注释不构成来源日期证明。后续须在投影丢弃完整批次前保存实际输入材料，并明确typed批次与transport原字节的区别，不从股票/标题反造来源证据。

2026-09-12当前开发片已实现非测试本地入口、完整context/input与缓存快照、v2显式迁移、同事务任务锁/首概念begin/result，以及真实prepare的typed stop。原5项[本地运行与首阶段恢复测试](../../src/push_foundation/intent_store/chain_post_close_v2_tests.rs)先由session53084取得缺接口编译RED，后补正常来源/长正文、输入篡改、跨运行许可和持久事实损坏4项验收。首轮实现编译session4875发现两处数据库查询借用生命周期错误，最小修复后session45722实际29项中28通过、1失败，源码前后及日志摘要已核：9项本地测试8通过；原7项schema和13项context/parser/非HTTP准备回归全部通过。唯一失败是旧head请求先返回LeaseHeld而非StaleLease，正在修正分类；该用例失败点之后的直接到期断言尚未运行，不能计作覆盖。首阶段重开读原结果、缓存变化不重复provider、真实提交失败停止、公开输入变化零begin均已有通过证据；不是本片全绿或Task2完成。

上述29项为前一轮记录；最新session3226（2026-09-12 01:48:47Z–01:51:32Z）实际32项中31通过、1失败，编译2分31秒、测试5.81秒，48条告警，570项源码前后/当次当前摘要及日志SHA一致。旧head分类及其后直接到期断言、有限浮点精确恢复、真实Pending future取消与重开零重发均通过。唯一失败是两个自有临时库在任务、owner、generation、head与请求相同而run_id不同时，A的调用凭据未被B拒绝；缺陷定位于RunLease/ConceptProviderCall缺少不可变运行编号绑定，正由原作者修复。没有将有限精度案例称为已复现缺陷，也没有将跨库测试称为生产事故。

最新修后session64782（2026-09-12 02:01:21Z–02:04:05Z）终态exit0：同一32项全部通过、3392项过滤，编译2分30秒、测试6.24秒、48条告警。唯一源码变化为RunLease/ConceptProviderCall携带真实RunId，并在签发、恢复、开始、结果及数据库任务锁校验处绑定；失败测试、SQL、codec及摘要算法未改。570项源码前后/当次当前与日志SHA独立复核一致，跨运行反例实际转绿，不将测试预期放宽。日志位于本计划执行记录的local-first-concept-integrity-green.log。

同一源码的消费者检查session97458（02:06:45Z–02:08:39Z）Clippy退出0，仅编译lib、stock_analysis和monitor，不运行二进制。570项源码及日志摘要核对一致，局部rustfmt检查通过。库197条、monitor 2条告警；相对原库188条增加的9条指向本地context/config构造、raw provider/clock/source constructor尚未接实际入口，不称零新增告警，也不为消除告警删除正常接线接口。

尚未迁移的缓存写入/后续provider/模型/报告/发送仍显式停止，不将临时StageNotMigrated当最终交付。接下来保持原并发上限6和完成顺序，补完整概念批次与同库缓存写入进度。完整Task2独立审查和Task3–4均未完成。

完整批次后续已开始：新增[贯通测试](../../src/push_foundation/intent_store/chain_post_close_concept_batch_tests.rs)由真实prepare进入一个缓存命中、两个missing的处理，要求原raw/同库cache进度、完整概念图与重开零查询/零重写。session65690（02:25:58Z–02:27:33Z）终态exit101，实际7个缺接口编译错误；571项源码前后/当次当前及日志摘要一致，未执行断言，不冒称测试已通过。具体v3一张缓存应用事实表/三个guard已核，完整布局31对象且原v1/v2保留；正在实现，尚未取得该片GREEN。此前32项通过对应前一冻结点，不代表当前新增测试已能编译。

2026-09-12完整批次首次实现验证：session60795（02:49:09Z–02:52:05Z）终态exit101，33项32通过、1失败、3392过滤，编译2分39秒、测试8.10秒、48条告警；572项源码前后/当次当前及日志SHA相符。新增贯通例实际通过：两份原结果/缓存应用事实持久保存，重开后零provider、零缓存重写。唯一失败为旧v2请求隔离例：第二missing被通用批次校验放行。修复方向是同事务核定真实布局，v2保持首阶段，v3才开放完整批次；不改原断言或SQL。六并发/乱序、普通错误收齐、多在途取消/结果提交失败、缓存原子回滚和v3迁移故障仍待补验，不据首例关闭Task2。

版本边界修后session49295（02:54:45Z–02:58:05Z）终态exit0：同33项全部通过、3392过滤，编译3分04秒、测试7.43秒、48条告警。只有facade/schema两文件修改，同一begin事务验证真实布局，v2只许固定首missing，v3允许完整固定missing；测试预期、SQL和codec未变。572项源码前后/当次当前与日志SHA复核相符。继续补真正六并发/乱序和普通错误收齐等验收，之后还须完成多在途故障、缓存/迁移回滚与持久事实关联完整性；本结果不是后续新增代码、完整Task2或生产运行的通过证据。

前三类补验session75563（03:05:25Z–03:08:27Z）终态exit101：36项35通过、1失败，编译2分45秒、测试8.74秒、48条告警；572项源码前后/当次当前及日志摘要相符，仅新增测试文件变化。全命中完整缓存图、8个missing真实poll/wake最多6在途、按[5,1,6,0,4,2,7,3]完成顺序逐result持久、全部完成前业务cache和应用事实均为零，已实际通过。普通错误测试首次error.to_string原因断言失败：现有PreparationFailure的Display按设计脱敏，应通过公开stage/reason核对保留原因。此处修正测试观察位置，不改生产脱敏；该测试后续raw/cache/重开与空raw子情形尚未执行，不先计通过。下一组将与多在途取消、真实提交故障、缓存回滚及partial三态一并验证，迁移/事实完整性等完整Task2剩余边界保持。

最新故障组session14534（03:22:59Z–03:25:57Z）终态exit0：40项全部通过、3392过滤，编译2分38秒、测试12.15秒、47条告警；572项源码前后/当次当前与日志SHA一致。此前session10770只有新增测试Result误作Error的E0599，未运行断言；修正解包后本组才获得完整通过。生产/SQL未为本轮改动。

本组直接证明：普通BusinessError和空raw原样全部保存，之后只写错误前的cache前缀，重开零provider；六pending取消/受控读锁故障不补位、同adapter再入和重开均停止；2raw+1cache边界的真实读锁争用后只保留原cache/fact/head=5，重开补未应用cache且保留已有行的live mutation与updated_at；partial明确区分Confirmed/BegunUnconfirmed/NeverStarted，有未决先停、无未决只补从未开始请求。cache公开错误目前仅AuthorityRejected，以上是边界/原子回滚证据，不冒称已直接读取底层commit操作类别。

下一组先补安全存储cause保留及持久事实关联反例（原请求/code/ordinal、跨事实version、可证owner/generation/time）；全部在owned临时库，不引入身份平台。独立cache SQL写故障、v2既有事实迁移/v3迁移回滚和封存拒绝仍待；阈值/聚类落库、其余来源/模型、artifact/报告/通知、Task3–4和完整Task2审查均不因此关闭。

2026-09-12事实关联反例session20424（03:37:59Z–03:41:26Z）终态exit101：45项38通过、7失败，编译2分59秒、测试17.32秒、47条告警；572项源码前后/当次当前及日志SHA相符。两个故障例的新增断言证明真实存储cause未传出；五个独立坏事实例分别交换code/ordinal及对应内容、复用另一结果版本、构造同代异owner/超当前generation/早于结果的written_at，公开inspect均误接受。原schema恢复、内容长度和摘要仍正确，故不能仅以哈希证明关系正确。已授权最小实现修复，保留原停止分类/脱敏展示及合法跨代恢复，不改原测试或SQL，不引入用户身份平台。此轮不是生产事故或提交回滚逻辑失效证明；完整Task2及迁移补验仍待。

关联与原因修后session70253（03:47:50Z–03:53:33Z）终态exit0：同45项全部通过、3392过滤，编译4分58秒、测试28.52秒、47条告警。572项源码前后/当次当前及日志SHA相符，较失败20424仅[盘后存储facade](../../src/push_foundation/intent_store/chain_post_close.rs)改变，SQL/codec/测试不改。原停止类型及底层真实ChainPostCloseError通过同一anyhow错误链保留，两个读锁故障现已直接核到commit类别；读取按原ordinal/code/raw与跨事实版本、可证owner/generation/time拒绝五类坏关联，同时合法跨代cache generation[1,2]恢复通过。以上七个自然RED已关闭，不将更严格校验变成外部身份平台。旧事实迁移/v3实际迁移提交回滚、封存拒绝与独立SQL写故障正在补验，完整Task2仍未完成。

2026-09-12迁移维护组session58384先取得50/52：两例在测试快照的Foundation排序列误用schema_version处失败，未到实际迁移。只将查询改为真实version列后，session44708（04:25:22Z–04:32:45Z）同52项全部通过、3392过滤，编译6分23秒、测试41.00秒、47条告警；573项源码前后/当前及日志SHA独立相符，生产实现/固定SQL和业务断言未改。[实际迁移测试](../../src/push_foundation/intent_store/chain_post_close_v3_migration_tests.rs)证明同库两份原v2 confirmed/unconfirmed运行事实无损升级、原v2 reader拒绝新版、重开零provider；真实COMMIT争用时完整回滚全部新对象/登记并保留旧事实，释放读锁重开仍v2且后续迁移成功。同批独立cache SQL失败与恢复、封存/异名附着/未来/孤立/未封存拒绝也通过；52是测试数，不是完整Unit交付数。

同一源码消费者Clippy session75032（04:33:29Z–04:37:44Z）退出0，实际检查lib、stock_analysis和monitor，不运行二进制；耗时4分14秒，库197条/monitor 2条告警。573项源码前后/当次当前与日志SHA核一致，相对97458按告警标题+源码路径比较无新增或移除诊断种类；不是全仓零告警或生产运行验证。

下一片已固定实施合同并开始测试先行：复用现有本地adapter，显式保存真实聚类阈值、首次clusters/aliases/成员顺序和isolated；同业务事务完成chain_daily原upsert、最近10自然日DISTINCT生命周期与应用进度。候选分支必须直接传播硬停止，普通不可用继续原降级。聚类恢复尚未实现；其余来源/模型、artifact/Ready、文件/逐渠道发送及Task3–4不缩减。

[聚类首个贯通测试](../../src/push_foundation/intent_store/chain_post_close_cluster_tests.rs)已冻结并运行：session87295（04:43:35Z–04:48:37Z）退出101，9个缺接口错误（E0432一处、E0425一处、E0599七处），574项源码前后/当前和日志SHA一致，尚未执行业务断言。测试使用真实prepare/自有SQLite/显式clock，要求阈值2、一簇两成员与别名/孤立股、07-12/07-15/07-21计为3天而排除窗外和未来、同日其他概念保留；重开前修改业务行与新增历史，恢复应保持原材料/生命周期而不覆盖现时变化。生产实现/冻结SQL未改；v4候选正在纠正配置不能依赖尚未取得的概念图、前驱事实版本不能等同当前head两处设计问题，这不是生产故障复现。

v4候选上述两处已修正并核定，进入真实实现：配置只绑定已经存在的run/context/input，完整concept map连同额外缓存代码在材料阶段保存；新事实按当下head做CAS+1，历史配置/材料只要求是更早的真实引用。新增3张职责表和9个保护trigger，旧v1–v3原字节保持。此时尚无v4实现编译/迁移通过证据，不等于聚类或整个Task2验收完成。

聚类首例实现后session79018（05:11:13Z–05:18:00Z）终态exit0：原52加新1共53项全部通过、3392过滤，编译5分50秒、测试38.48秒、47条告警；576项源码前后/当前及日志SHA独立核一致。真实prepare保存阈值2、成员/别名/孤立股和原材料，chain_daily保留原同日upsert、10自然日DISTINCT计数与其他概念；重开返回原材料和生命周期3，零provider并保持之后的业务修改。该首例采用概念缓存全命中，不证明有missing时配置先于provider，也不替代配置冲突/空聚类/材料关联损坏/真实业务SQL和COMMIT回滚/v4旧事实迁移验收。材料写前精确往返、业务应用对原概念事实关联、父子事实代次关系的静态疑点将由既有合同补验落实，不将其称为已复现生产事故。v4消费者Clippy和完整Task2独立审查仍待。

新增[聚类维护测试](../../src/push_foundation/intent_store/chain_post_close_cluster_maintenance_tests.rs)先因四处测试父模块路径错误在session60741编译退出101，未执行业务。仅修导入/四引用后，session28463（05:53:10Z–06:01:17Z）实际67项64通过、3失败，编译6分45秒/测试60.68秒、47条告警，577项源码前后/当前和日志SHA核一致。原53与11项新增维护通过：max阈值持久重开/冲突、missing前配置与精确完整map、合法跨代/重叠/同涨幅原序、空聚类、业务SQL与真实COMMIT回滚/重开原材料，以及坏map/codec/lifecycle/version读取拒绝。三项失败证明应用前坏完整map未触发应有停止、父子代次/同代owner异常被inspect接受；各失败后面的保持/重开断言尚未执行。当前只修这些实际关联缺口，原测试/SQL/codec保持，不是生产数据事故；v4迁移、候选硬停止及其余Task2仍未完成。

恢复资格明确区分：已开始但结果未确认，先整批停止；已确认，只读重放；固定ordinal从未开始，可由明确执行入口继续新建begin，不能被误称Unknown。只读inspect不发请求。Task3仍须落实窗口内可开新工作、窗口外仅恢复已有事实的区分，本片不提前宣称该窗口门已接入。

安全范围须分清：新准备观察类型的Debug/错误展示及本模块改动日志只输出安全元数据；[analyzer/client.rs](../../src/analyzer/client.rs)的原有非2xx响应正文日志未在本Task整改，不能称整条上游日志已全面脱敏。此风险保留到后续安全工作，不借扩展本次源码范围掩盖。
