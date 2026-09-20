# 盘后龙虎榜：真实效果、接线次序与验收边界

## 最新增量：历史结果运行关联（2026-09-14）

[历史结果 reader](../../src/push_foundation/intent_store/chain_post_close_dragon_tiger.rs:353)现在读取并校验 run_id、run_context_sha256、input_sha256，与恢复出的原 run/原输入一致；原时间及投影校验保留，不改 SQL、codec 或普通来源策略。[新增表驱动反例](../../src/push_foundation/intent_store/chain_post_close_dragon_tiger_tests.rs:885)逐字段重建真实 v10 场景，损坏后实际 prepare/inspect 精确 SchemaRejected、零新 RPC/审计/缓存或日期观察，不自动修复。

63850 真实 RED：2026-09-13T17:48:09Z 退出 101，4 通过/1 失败；仅执行到 run_id，prepare=other_error、inspect=accepted。不得称后两字段也已取得 RED。日志 SHA 为 67950ec701d883a60b95149e083dc6982df0fa1fb450f5c0c9edd338a62d867a，613 项前后源码一致。

99257 合批 GREEN：2026-09-14T00:30:53Z 至 00:36:31Z，169 通过/0 失败/3344 过滤，测试 170.13s；保留原相关回归和原单个 skip，新增本片 1 个测试及生产根适配 2 个测试，不是全仓 suite。三字段案例均完成，正常真重开和原损坏反例同时通过。日志 SHA 为 8b285f4c3e326c467dae65896cae2e0363240aadd8f9e1a62e66ae2ca8a22ffb；614 项前后及本次核查时当前源码、日志摘要均一致，保留 56 条 lib-test 告警。

本片 journal SHA 为 fa66f47300dcf4ae0423f8a838495f008809aef07668c7f488d99ff62c01c1f9，测试 SHA 为 1558e2106af62282c74a08d94da30183303f5d9e942e05064e96e7e60ac5f32d。独立 Spec/Quality 静态审查通过；1 项报告措辞 Minor 已澄清（测试由主控先新增，实现 agent 未再改测试）。新消费者 Clippy 52257 于2026-09-14T01:40:19Z退出0，lib/stock_analysis/monitor通过，614项before/after/current及日志SHA6165611e4d64faf2d5a0bb695c77e113196dba0d24771080b8c5b7de8d5cef94核一致；lib231/monitor2条告警保留。本片定向开发验证关闭，不等于Task2或完整故障矩阵关闭。

用户现授权启动最新 monitor，必要的生产数据核查与备份已执行；实际启动状态见[原项目启动验收](/Users/zhangzhen/Desktop/Quant/stock_analysis/docs/push-system/releases/monitor-start-2026-09-14.md)。下文“未读取生产库/未运行”的表述限定各历史验证当时。完整 Task2–4/52 Unit 仍未完成，不将编译进 binary 等同生产接管。

日期：2026-09-13，更新至2026-09-14。状态：本轮修复历史结果时间校验，四项定向测试、166项相关回归及消费者编译检查通过，限定独立审查通过。此前准入后损坏等证据分别保留，完整故障矩阵仍待补齐。属于[盘后恢复计划](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)Task2；不缩减Task2–4、W15–W21和全部52个迁移单元，也不重新引入已排除的复杂可信身份平台。

## 已核定的原业务，不在迁移中改写

| 环节 | 当前真实行为 | 代码证据 |
| --- | --- | --- |
| 请求日期 | 调用时取本地带时区时间，用其自然日；不是pipeline业务日 | [fetch_lhb_observed](../../src/pipeline/chain_analysis/fetchers.rs) |
| 请求参数 | DragonTiger，披露上限100、股票上限5000；不能换成MarketDragonTiger | [Gateway](../../src/data_gateway/dragon_tiger.rs)、[gRPC来源](../../src/data_gateway/grpc_source.rs) |
| 请求身份 | market.dragon_tiger v1；同次重试共用原request_id、原请求字节，provider为空、不接纳诊断数据 | [信封](../../src/grpc_client/envelope.rs)、[冻结schema](../../src/grpc_contract/schema.rs) |
| 每次RPC | 每次重新附授权；默认最多4次，按原Status分类决定重试；首个退避1000ms；Response的信封/转换错误不重试 | [client](../../src/grpc_client/client.rs)、[重试规则](../../src/grpc_client/retry.rs) |
| 完整来源 | 保存股票及其每条披露、席位、可选数值、完整BatchEvidence；空数组才得到VerifiedEmpty | [原转换器](../../src/data_gateway/grpc_source/convert.rs) |
| 采集审计 | 整次查询终态后写一条R-04审计，不是每次RPC写一条；成功取批次provider，错误回落Eastmoney | [Gateway](../../src/data_gateway/dragon_tiger.rs)、[审计映射](../../src/data_gateway/review.rs) |
| 准备投影 | 净额从元除以10000变成万元；Gateway普通失败降级Unavailable，非法代码/非有限净额/重复代码的投影失败仍阻断 | [fetchers](../../src/pipeline/chain_analysis/fetchers.rs) |

特别边界：原DragonTiger通用转换器没有额外校验响应payload的schema/version/content_type；迁移恢复必须复用原转换器，不能悄悄增加准入政策。请求身份校验与响应业务准入不是一回事。客户端源码不足以证明服务端一次RPC内部的HTTP/provider请求数，不将“客户端最多4次”推广到整个服务端。

## 本次正在实施的接点

原GrpcSource::dragon_tiger_async只返回内部全部重试后的批次，持久调用者无法在每次实际RPC前后记录。因此先扩展已有closed attempt执行器，提供具名龙虎榜session、原request续接、原Response/Status只读恢复，并让旧公开dragon_tiger_async消费同一接口。保留原板块/概念方法及其wire，不新增任意Operation/JSON的通用工作流接口，不复制一套重试政策。

首例经真实tonic loopback验证：第一RPC返回Unavailable后立即交还控制、原1000ms退避、丢弃session后按原request续接第二RPC、完整Response/披露/席位/证据保持，以及旧公开入口仍同策略成功。这里只证明“session重建”和协议恢复，**不是数据库重开恢复**；受控请求日期也不能替代实际自然日時钟的持久验收。

## 完整龙虎榜阶段合同（实现与验收分别记账）

1. 在同业务库固定龙虎榜运行/父阶段身份、带offset的首次请求时刻和派生自然日。持仓为空、全缓存命中、第二批RPC补齐三条父分支都要可证明完成；仅v9事实读取合法不等于该阶段已完整。
2. 存原operation/request_id/protobuf字节、profile、来源authority和四项重试策略，但不存bearer。每次授权成功后、RPC await前提交begin；每次返回立即提交原Response或Status细节，再决定下一步。
3. 原Status code/details/二进制error-detail trailer（区分缺失/损坏）、RetryDecision、实际退避与Terminal分别保留。耗尽重试不能把原RetryBackoff/RetryBounded篡改为NoRetry。
4. 成功/验证空/错误终态保存完整原批次或可逆错误；同连接事务写R-04审计及真实receipt，再保存原准备投影。重开不可重复审计，不可只保存净额map丢掉原披露/席位。
5. 仅有begin没有result保持未决并阻断；提交确认丢失先只读查证，已存Response只恢复本地转换/最终事务，不再RPC。取消保护不能在Drop中写库。
6. 用受控新增布局登记及迁移保留v1–v9原对象/事实；验证/重开不自动迁移、不自动修复。坏引用、跨运行材料、过期执行者、CAS漂移和不合法阶段顺序拒绝。
7. 接入真实prepare并消除该阶段的StageNotMigrated；下一未迁移效果仍显式停止，最终还须接完宏观/搜索/模型/报告/发送，不得回落到未记账ProductionIo。

## 历史结果时间关系修复（2026-09-14）

确认并修复了一处恢复校验缺口：[原SQL插入守卫](../../src/push_foundation/intent_store/chain_post_close.v10.sql:614)要求“请求开始 ≤ 结果返回”，但原历史读取没有读取返回时间。仅在测试库改坏该时间列后，已准入对象的inspect接受了矛盾记录，prepare也未返回预期的SchemaRejected；不把其other_error误说成完整准备成功。

[修复](../../src/push_foundation/intent_store/chain_post_close_dragon_tiger.rs:680)只增加返回时间读取，并验证“开始 ≤ 返回 ≤ 提交”，保留提交不晚于运行更新时间的检查。没有修改SQL、codec、RPC重试或普通来源降级政策。[回归用例](../../src/push_foundation/intent_store/chain_post_close_dragon_tiger_tests.rs:881)复用已持有facade/IO的真实场景，第二连接只将returned_at改为0，完整恢复原trigger；其余事实、审计与schema保持。实际prepare和同facade inspect分别精确拒绝，零新RPC/采集审计/请求日期或缓存观察，无自动修库。

| 验证 | 实际终态 |
| --- | --- |
| 70992 初次反例 | exit101，3通过/1失败；唯一时间反例为prepare=other_error、inspect=accepted，原三个用例通过 |
| 80508 修后同组 | exit0，4通过/0失败，3506过滤；测试24.50s，编译3m01s |
| 80583 原范围增量回归 | exit0，166通过/0失败，3344过滤；测试156.22s，编译1.25s；argv与此前64823相同，保留原单个skip |
| 限定独立审查 | Spec/Quality通过，本片0项Critical/Important/Minor；不代表全仓无告警 |
| 消费者编译检查 | 87868于17:36:35Z退出0，lib/stock_analysis/monitor检查通过；1m44s，lib231/monitor2条告警；未运行可执行文件 |

三个测试捕获各613项before/after与日志SHA已独立复核；RED对应修前冻结点，两个GREEN与最终源码一致，均保留56条lib-test告警。RED→GREEN只有journal变化，测试预期未改。当前journal SHA为fde73553457ab24553bc2c69ca924ea631c2ba5bbd9f91d20865fa7d81662ebf，child SHA为957a7e17d7b38f56d58075a14fa0f8c98400da028a4d5790110e2bc26c987c5f。完整命令及时间见开发树本计划私有目录的local-dragon-tiger-timeline-*捕获；日志SHA依次为e201e7bf7c16199d5f74343089797bcb14d92fdd3d6b660aab1a2af76aeb9722、95d397d1336bc6346ffe10c3749624183001d0bf605dc94f4af12a05d9627471、3a72dd844a15716b454933b2f9c7c7fae0b4582aae23e7df4da1e22d3dcc7c32。

87868的613项before/after/current与Clippy日志SHA eb24279b4e8518523039a36d7af892ddc11dc060cad1b3c401196410322090a7核一致。与此前34666按告警标题、源文件及重复数量比较完全相同，233条既有诊断保留；不称零告警，也不将编译通过等同实际monitor运行。

独立报告SHA为70e407299c84088758896d64ceb44ab642b4034f09c8f9020915164aaaa016fe；其记录时未核80583终态，现已由主控实际补验，不重复审查相同差异。此处只关闭一项历史时间缺口；历史运行/owner/代次/父链、Status/重试/提交等完整矩阵和后半段业务仍未完成，不计作整个Unit迁移完成，也不作为首批手动包前置。

## 前次准入后损坏补验（2026-09-14，96217冻结点）

新增[已持有运行对象后的损坏用例](../../src/push_foundation/intent_store/chain_post_close_dragon_tiger_tests.rs:876)：先由真实prepare保存合法v10结果，重开后在数据仍健康时取得同一facade和准备IO，再用自有临时SQLite的第二连接改坏投影。损坏仅将12.5万元改为13.0，长度/hash保持自洽，原trigger精确恢复；随后真实prepare与同一facade的inspect分别返回SchemaRejected。没有重新构造facade借准入拒绝代替实际调用；没有新增RPC、采集审计、请求日期或缓存观察，也没有隐式修复原事实。

96217/local-dragon-tiger-held-facade于2026-09-13T17:19:14Z退出0：三项定向测试全部通过（含原正常真重开、原F2和本例），3506过滤，编译2m54s、测试19.57s、56条lib-test告警。613项输入before/after/current及日志摘要已核一致；log SHA为47b0db4b865d30179205df9b4d57e197a0df16f452d72e280bd60cb9470fc192，测试child SHA为a5e4e78de8a0f2958047fd269b01947169efa5b8fa43e77b5734721519736983。

本轮只改测试文件，原F2生产修复保持；新例首次自然GREEN，不声称经历了新的RED→生产修复。先前164项及Clippy的生产输入未变，但本轮没有新增后的165项合批或新独立审查。此处补齐的是准入后损坏路径，不是完整历史关系、错误/重试/提交矩阵，也不作为首批手动包上线前置。

## 验收矩阵与此前F2证据

2026-09-14最新增量：F2仅改journal，从原terminal/request重建Gateway和投影，完整final比较及inspect校验已接通。53646曾在facade构造处提前SchemaRejected，使原测试unwrap前提失败；随后只修测试，两个受约束路径分别严格识别该拒绝。26431/local-dragon-tiger-v10-f2-admission-fix于2026-09-13T16:53:21Z退出0，2passed/0failed/3506filtered，编译2m57s/56告警、测试12.48s；log SHA4d75b1726c9ffb6360e9ccf3c6cc9078e97233ba7f383d9b0e1981f10edd1864。64823/local-dragon-tiger-v10-f2-consumer-regression于16:57:49Z退出0，与原21077精确同argv，164passed/0failed/3344filtered，编译1.19s、测试158.48s/56告警；log SHA120e9f3568ea19e417834c2577c8e95f9e4aa7739b5448068bdd1d261d8e8bce。两次各613项before/after/current及log摘要独立一致，journal SHA002646856690c6c63246290fbdbc8d0e39ef57feef870ade96d5ed0454281017、child SHA73c01ea13d3a55a754031bf362318424b7fe1110c0d52595c730c75d77da42bf。F2限定Spec/Quality通过，0项Critical/Important，1项既有告警Minor；当前反例可在facade准入提前拒绝，不等于持有facade后再损坏的深层校验。34666/local-dragon-tiger-v10-f2-consumers-clippy于17:01:49Z退出0，同源lib/stock_analysis/monitor检查通过（未运行），1m42s、lib231/monitor2告警；613项before/after/current和log SHAe657f943b4f52e7cd8b4698c6d1b4070c70d51139e15dea2485c35cc67884d47核一致。与95400总数同为233，但8条未使用接点告警消失、8条v10/版本相关告警新增，不能称无新增诊断；新增journal警告对应的原代码在F2前像已存在，后续Task2维护仍保留。完整历史行关系、错误/重试/提交/迁移矩阵、后半段业务接线及切换仍待。原18058的inspect=accepted/prepare=other_error是真实RED，不把other_error说成整个prepare成功；过程记录保留于本计划私有ledger。

| 验收 | 必须证明什么 | 当前状态 |
| --- | --- | --- |
| 逐次协议与旧入口兼容 | 原请求重试、原wire/分类、旧公开消费者同策略 | 67604首例通过，74652的162项回归保留该例及原160项；95400同源消费者Clippy通过 |
| 恢复输入拒绝 | 日期/limit/schema/id/profile/authority/次数错误，零RPC | 待实现验证 |
| 原失败策略 | 授权失败零RPC，Response错误零重试，Status重试/耗尽保持 | 待完整矩阵 |
| 请求自然日 | 业务日7月21日、请求本地7月22日；重开改变时钟后仍用首次事实 | 1734首例通过；39765同源码回归中仍通过 |
| 正常真正重开 | 临时SQLite关闭重开，公开prepare保留完整批次及万元map，零新增RPC/审计 | 1734首例通过；全部父分支/错误/故障矩阵仍待 |
| 提交/取消/Unknown | begin/result/终态事务各故障点，零盲补发、零后续效果 | 龙虎榜逐点故障验收待补，不能借板块故障例代验 |
| 迁移/关系完整性 | 旧对象事实不改，历史顺序/父链/范围拒绝，失败不修库 | v10迁移首例与F2自洽投影损坏拒绝通过；历史链/错误/完整迁移故障矩阵仍待 |
| 准入后出现损坏 | 健康时取得facade/IO，第二连接改坏后实际prepare/inspect拒绝且无隐式修复 | 96217三项定向测试通过；完整历史时间/身份关系仍待 |
| 历史时间先后关系 | 已保存结果不能早于请求开始，且返回不晚于提交 | 70992真实反例→80508四项及80583相关166项通过；完整历史身份/其他阶段关系仍待 |
| 独立审查与真实接管 | 限定实现审查、完整Task2审查、调度/切换分别有证据 | 逐次RPC接口和F2限定Spec/Quality通过；完整v10/Task2审查及实际接管仍未完成 |

验证身份：67604于08:04:57Z退出0，1项通过；74652于08:08:52Z退出0，162项通过、0失败，测试165.44秒；95400于08:18:31Z退出0，仅编译lib/stock_analysis/monitor。608项输入在三次检查前后及本次记录时一致；测试日志SHA分别为07e827265b3bc31a9ec6132febc5a8abde97227588bfe266bd394825f55a6285、a070ccb00d4b41df4bd660f2a9b0dcbee0d42013c3aa31b92f04e7fc270e7f64；Clippy日志SHA为cf05b8bfe22548f484fea6864498d77fab8857c09d759a96b8c1a7dff3efd215。测试保留52条告警；Clippy保留233条，较8691净增10条（7条尚未被正常持久消费者使用的恢复接点、3条参数数），不报零告警。独立审查建议后续集中重复连接取得代码，未发现阻塞项；这些局部证据不证明数据库重开或实际发布。

全部测试只用显式实例配置、合成内容、随机本机loopback和自有临时库；主控独占单Cargo队列。当前未运行真实来源、读取生产数据库、启动/观察monitor或替换原程序。
