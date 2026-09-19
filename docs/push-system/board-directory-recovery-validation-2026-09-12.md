# 板块目录持久恢复：验收范围与当前证据

日期：2026-09-12。归属[盘后实施计划的 Task 2](../superpowers/plans/2026-09-11-chain-post-close-recovery.md)。这是已批准行为的验收清单，不增加业务政策，也不代表下表测试已经实现或通过。单用户范围以[范围调整](single-user-local-scope-2026-09-11.md)为准；无可信身份平台、生产迁移或 monitor 操作。

## 当前证据

- 最新开发状态（2026-09-14北京时间）：已修复龙虎榜历史结果时间校验缺口。70992真实反例后，80508同四项、80583原范围166项回归及87868消费者Clippy均通过；限定独立Spec/Quality通过。源码/日志摘要已核，保留56条测试告警和233条消费者告警，不冒称全仓无告警或完整Task2通过。完整历史身份/Status/重试/提交矩阵、Task2–4/W15–W21/52Unit仍未完成。首批手动包不等待本片，运行根方案尚待确认，本轮未启动或替换monitor。详见[龙虎榜恢复验收](dragon-tiger-recovery-validation-2026-09-13.md)。

- 前次板块Retry续接验证（2026-09-12）：session50677于11:47:38Z退出0，105项全部通过；编译5m00s/测试65.22s、49告警，585项源码前后/当前与日志SHA一致。真实Retry确认后取消/重开，完整退避后以原request/次数/策略续接，原首组事实不改、跨代引用保持、最终总3RPC与两条原字段BR159均通过，关闭68550反例。6个正常文件修改，原测试/Cargo/固定SQL/codec形状保持。消费者Clippy session64389于11:52:07Z退出0，3m58s、lib202/monitor2告警，585项及日志核一致，较17868无新增/移除诊断种类；仅编译未运行monitor。该冻结点尚未覆盖随后新增的实际授权失败用例，当前结果以上条为准。完整授权/故障/兼容矩阵、错误终结材料、Task2/真实调度和52Unit仍未完成。

- 前次成功Response提交恢复验证（2026-09-12）：session25581于10:56:05Z退出0，104项全部通过，编译5m56s/测试72.74s、49条告警；585项before/after/当次当前与日志SHA独立一致。真实Concept最终COMMIT争用后回滚，关闭重开换代后只用原Response补最终事务；总RPC仍3、原begin/result及Industry旧审计不改，两条最终BR159的完整原字段/时间/批次均保持。消费者Clippy session17868已于11:01:11Z退出0，编译3m54s，lib202/monitor2告警；585项源码前后/当前与日志SHA一致。仅编译未运行二进制，新增三类诊断已记实施台账，非零告警验收。ConfirmedRetry、错误终结及其他故障/兼容矩阵、完整Task2/真实调度/全部52Unit仍未完成。本轮只5个正常Rust文件改变，固定SQL、104项测试及codec序列化形状保持；成功Response提交恢复不代证Status/错误fallback或ConfirmedRetry续接。
### 历史过程（按各次冻结点保留）

以下是此前诊断和实施记录，不代表当前尚未修复；当前结果见上方首例与合批进度，剩余范围见下方验收矩阵。

- 最新诊断：session68422于09:50:32Z退出101，585项源码和日志SHA已核；安全原因是`StorageFailed("board begin fact")`。实际代码把attempt ordinal编码进逻辑request_bytes，第二次重试改变摘要，而固定v5 guard要求同逻辑请求的摘要稳定。已授权最小修复：ordinal仅属真实尝试事实，完整原请求/策略保持稳定，reader另核连续前缀及同请求身份；固定SQL与原业务断言不改。尚无修后通过证据，临时测试诊断将在首例闭合后移除。
- 最新实际运行：两文件接线修正后，`local-board-directory-vertical-compile-fix` / session98938（09:11:20Z–09:17:46Z）完成编译，但测试在绑定本机端口时PermissionDenied。用户批准原精确合成loopback测试后，`local-board-directory-vertical-loopback-permitted` / session23807 / PID22239（09:21:41Z–09:21:49Z）退出101：编译3.48s、测试3.17s、50条告警，失败于board_tests.rs:69，返回错误未匹配PreparationStop::StageNotMigrated(Positions)。两次585项before/after/当次当前及日志SHA均独立核一致。尚无更下层原因证据，不能说业务已通过或继续归因沙箱；先增加必要安全诊断，原断言和业务要求保持。
- 最新原贯通候选验证：`local-board-directory-vertical-green`，session55777 / Cargo PID18559，2026-09-12 09:03:31Z–09:08:05Z，退出101，585项输入before/after/当次当前与日志SHA均独立核一致。5个E0425来自目录读取行错放导致四处局部变量缺失，以及PositionInput引用层级错误；4条编译告警。已交原作者仅修两文件，原测试/SQL/codec不改，未执行业务断言或loopback。已知terminal raw→final及ConfirmedRetry续接缺口仍在后续故障例范围。
- 上一冻结版本的审计与盘后回归：session76721，90 项通过；消费者 Clippy session1348 退出 0。两者均早于本次板块接线，不能作为新实现通过证据。
- [真实板块贯通测试](../../src/push_foundation/intent_store/chain_post_close_board_tests.rs)：session8214 编译退出 101，未执行业务断言。7 处 E0599 中，6 处是正常接口缺失，1 处是测试 Diesel trait 导入问题；后者仅修导入，原业务断言不变。正常接口正在实现，尚无修后通过结果。
- [v5 SQL](../../src/push_foundation/intent_store/chain_post_close.v5.sql)已落盘，42,017 字节，SHA-256 为 `b2c48142faf90409b7d54d028a2deecf7f62a9f3665c2087dfb3961e3f521918`，与核定草稿 SQL 围栏内容逐字一致；v1–v4 摘要未变。
- 主控将五份实际 SQL 依次加载到 SQLite `:memory:`，SQLite 3.51.0 执行退出 0：新增板块表 5 张、trigger 15 个，盘后扩展有定义对象共 63 个。这只证明该 SQLite 版本接受 DDL 及对象数量，不证明 Rust 迁移、数据约束、运行接线或项目所用 SQLite 的兼容性。
- 实施中静态核对发现新请求先附加授权、再 `into_inner` 丢弃 metadata，发送时重建无授权 Request；已反馈原实施者修正，并要求共享原 RPC 路由/重试语义、保留错误 trailer 的缺失/有效字节/损坏三态。尚未取得修后运行证据，不称生产事故或已修复。
- 新审计准入实施中的兼容反例：从 Task 2 基线 `4da61ad` 提取原七条建表语句，与当前抽取常量、冻结目录 fixture 分别建三个独立内存库。冻结 fixture 与基线的 8 个对象（含自动索引）完全一致；当前版本 7 个有定义对象的 SQL 字节均不同，原因是抽取时改变了 SQL 内部空格。检查时审计模块 SHA 为 `5734e2d96359880ecdb9a917d437c2bc5d0f98cf7e3d72c211e1f288e15d6f9a`，检查前后该文件未变；结果已反馈作者。不能把测试库也改用新常量后自比成功当作旧库兼容，也不能通过归一化 SQL 来放宽原精确合同。
- 上述 DDL 兼容反例已纠正并复验：恢复原逐条建表语句后，同一三方检查退出 0，基线/修后实现/冻结 fixture 各 8 个对象逐字一致、差异为空。修后审计模块 SHA 为 `4c68ec4b9d77ac275ca86a8b02287c9a132ab22cb11ff3a76f234bae59ed68a3`，检查前后未变。本结果关闭的是 DDL 字节漂移，不代表新准入函数、板块恢复或原 Rust 回归已通过。

## 首例通过后的验收矩阵

v6 当前补充证据：新增[两张材料表的 SQL](../../src/push_foundation/intent_store/chain_post_close.v6.sql)与 v1–v5 依次加载到独立 SQLite 3.51.0 内存库，执行及外键检查退出 0（扩展共 18 表、53 个 trigger）。仅抽取 A 表实际 CHECK 的六个独立 shape 检查中，两个合法 Captured/Legacy 对照接受，四个关键字段 NULL 变种均拒绝。所测 v6 SHA-256 为 `ab520eb71a62ea476777ab525f1fb2f52823c0d035b7defcebc8834c0cea945e`。这些不是 Rust 回归、真实迁移或恢复验证，不计入通过测试数量。

正常实现已收口运行身份绑定、材料版本/安全诊断校验、所有 Status 对 A 的完整覆盖，以及成功批次 provider 保真。首轮128项中的终结错误用例只使用 Tdx；后续 session79493 已完整通过 [Custom provider回归](../../src/push_foundation/intent_store/chain_post_close_success_provider_tests.rs)：v5/v6首次及真正重开保留完整目录、raw protobuf、批级/逐记录provider及原完整BR159，零重复RPC且原事实不改。成功审计使用原批次 provider，错误审计保持 Tdx；不是完整v6故障矩阵验收。

原正常重试/重开、成功Response最终COMMIT恢复、已确认Retry取消/续接、恢复凭据非法、result COMMIT未决及begin COMMIT回滚六个子例已有通过证据；65363再验证v6终结Status的原错误/首次fallback恢复，76293再验证B COMMIT未确认后的同实例停止与真重开零重发，共八个已证子例。它们不替代最终审计行/链SQL、错误Response、协议/耗尽、其他授权、未决/过期与迁移/损坏完整矩阵。继续合批相关场景，不以测试数量代替完整Unit交付。

| 场景 | 必须观察的业务结果 | 证据入口与隔离要求 |
| --- | --- | --- |
| 正常重试及重开（首例通过） | Industry 两次、Concept 一次真实请求；三组逐尝试记录，但最终只有两条 BR159 审计。首次选择和候选 Unavailable 保留；重开零请求、零审计/原材料重写，在尚未迁移的 Positions 停止。 | session86171原贯通测试通过，真实公开 prepare、真实 client 和自有 tonic server；不改成直接 store CRUD。 |
| 请求构造、授权、begin 失败（恢复实例凭据非法与begin COMMIT子例通过） | 构造/授权失败不产生实际 RPC begin；begin SQL 或提交失败时 server 零请求。实际发送的 Request 保留授权，数据库不保存凭据。 | session42218同108通过：共享错误上下文修复关闭43549的真实intent RED；原commit cause/Candidates及零RPC、释放后fresh回滚、真正重开换代正常首次执行、两条原BR159完整字段及总3RPC均已执行，原授权停止保持通过。其他构造/授权及begin SQL仍待。 |
| 请求返回后的 result 失败或取消（COMMIT后重开零重发子例通过） | 已提交 begin 的实际请求发生一次；结果 SQL/真实 COMMIT 失败或 future 取消后，没有确认结果、后续请求为零。同 adapter 再入和重开均明确未决，不盲重发。 | session43549中新result COMMIT例完整通过：真实begin确认/Industry一次RPC后第二连接读锁令实际result COMMIT失败，保留ResultUnconfirmed及commit cause，只有原完整begin；释放后fresh读取及真正重开/合法换代，公开prepare以IncompleteOnReopen停止且零重发/零补造。独立SQL、取消、同adapter再入及其他未决场景仍待；不能以此单例替代。 |
| 最终审计提交失败（成功Response COMMIT子例通过） | terminal raw 已确认，BR159 audit/chain、kind-final、run head 必须一起回滚。重开只用原材料补最终事务，不重新请求；错误回退的首次 observed_at 不变。 | session25581修后104项全部通过，关闭33118恢复RED；真实Concept COMMIT争用、回滚、重开后Positions/原目录/选择/两receipt全部字段及总3RPC均已验证。审计行、审计链、最终事实SQL失败和错误fallback仍需覆盖，不以成功Response子例替代。 |
| v6 已确认终结错误恢复（Status子例通过） | Status 原安全 diagnostic 与 raw 同事务确认；首次 GatewayError 六字段、首次 fallback 与完整 BR159 原记录另行确认。final COMMIT 失败后重开只补最终事务，完整错误、首次时间不漂移；再次重开零重写/零 RPC。 | session65363中terminal_error_tests完整通过：真实prepare/FailedPrecondition、B后final COMMIT争用、fresh回滚查证及两次合法换代真重开，六字段与首次审计时间07:31:03.456Z保持，总RPC仍3且原事实不改；完整BR159 reader和独立字段同时核验。该批唯一失败在飞书，不是该子例。错误Response、协议冲突、耗尽仍待。 |
| v6 错误材料未确认（B COMMIT子例通过） | raw 已确认但首次错误材料未确认时，重开不重采时间、不重发、不补造。首次材料只接受实际 raw COMMIT 后的私有一次性能力；取消或确认失败不重新签发。 | session76293中[公开prepare故障例](../../src/push_foundation/intent_store/chain_post_close_error_material_commit_tests.rs)完整通过：预开读锁使B实际COMMIT失败，真实intent/commit cause/Candidates保持；raw+A和原Industry审计完整保留、head不越过A，释放后fresh回滚查证，同adapter无lease先停止，真重开合法换代后IncompleteOnReopen且零B/RPC/新增审计。独立B SQL、取消与raw确认后采样前崩溃仍待；这些窗口不能称自动终结。 |
| 响应后置验证失败 | request_id/authority/信封/payload 转换失败时仍保留实际解码响应及各 payload 原字节；按原普通失败政策处理，不新增原流程没有的重试，不伪造 transport 响应。 | server 返回合成异常信封；区分重新编码的 protobuf 与真正 wire framing，不将两者混称。 |
| 原错误协议与重试 | 显式 retryable=false 禁止重试；允许重试的错误、默认无 detail 的运输错误和不可重试的请求/权限错误遵循原表。标准 details 与特殊 trailer 的一致、冲突、损坏、缺失均保留原判断；同逻辑请求 ID、总尝试上限和退避不漂移。 | 复用原 errors/retry；真实 server 的请求序列与持久错误材料交叉核对，不保存任意 metadata 或不安全正文。 |
| 行业提前终止与字符串语义 | Industry 普通错误、VerifiedEmpty 或同一行业批次内同名不同 code 冲突时不请求 Concept。同名同 code 可合并；原字符串不加 trim/512 限制，选中 Some(空字符串) 与 None 不混同。 | 真实 prepare 的受保护观察和 server 计数；候选仍是现有 unsupported→Unavailable、零候选 RPC。 |
| 首次实际选择 | 在原逐 cluster 解析点保存首次选中 code。存在多个模糊匹配时，恢复使用第一次实际结果，不能重新跑 HashMap 解析再比较；已确认选择与顺序不重写。 | 首次正常运行生成事实，再关闭重开；预期取已保存的首次真实选择，不由被测恢复函数反造。 |
| 接管与未决边界（Retry续接子例通过） | 已确认可只读重放；已开始未确认先整批停止；上一重试结果已确认、下一次从未开始，才允许按原请求及策略继续。旧 lease、到期后返回的写入被拒绝，合法跨代父子关系保留。 | session50677：修后105项全部通过，关闭68550反例。真实Retry已确认后取消/重开，999ms前零新begin/RPC、完整退避及Tokio至多1ms精度余量后原request/ordinal/policy跨代继续；旧首组不改、最终总3RPC/两审计全字段保持。授权失败、其他未决/过期等仍待，Task 3 真实窗口另行实现。 |
| 板块准入与定点审计核验 | 缺表/坏链/定义漂移/异名附着、TEMP 或其他库遮蔽不能放行或自动修复。板块 factory 在同连接读事务中准入完整链；无准入的 inspect 先准入或拒绝。逐 receipt 不反复全历史扫描。 | 原 BR159 schema 与链算法；至少一份旧库来源于冻结 fixture/基线，不能仅用修改后的 create_schema 安装库并自比。定点核验对照旧完整链 reader。性能边界同时检查真实调用位置和 SQL 范围，不能用小样本通过推断历史规模性能。 |
| v4→v5 与旧行为兼容 | 含真实 v4 已确认/未决事实的库迁移后旧字节、旧登记及业务行保持；实际 COMMIT 失败完整回滚新增对象和登记，重开仍为 v4，可再次迁移。旧精确版本 reader/factory 不被改成任意新版通吃。 | 固定 SQL 独立 reference 对照实际目录；旧 v4 不带 BR159 的原回归仍可用；精确 v5 factory 继续只接 5，新 v6 用独立 factory，generic reader 仅接纳实现明确支持的完整布局。 |
| v5→v6 与旧缺材料见证（真实成功链迁移及COMMIT回滚子例通过） | 新 DDL、registry、LegacyV5Absent 与 seal 同事务；旧 Status 见证绑定原 PK/version/SHA，不伪造 diagnostic、时间、lease 或新业务版本，旧 raw/头/定义保持。旧成功 Response/可验证 Retry 可恢复，旧终结错误缺完整原材料则明确停止。 | session89312的两个[迁移测试](../../src/push_foundation/intent_store/chain_post_close_v6_legacy_migration_tests.rs)通过：真实v5 Retry后成功链、Status/Response/完整审计原事实不改；新A精确Legacy形状、实际COMMIT回滚/fresh视图/真重开仍5、再迁6及新factory零RPC重放。旧exact5拒6。迁移时仍停在Retry、旧terminal缺B、半装/缺表/未封印/foreign与新Status缺A等其余矩阵仍待，不以两个子例代验。 |
| 事实关联与损坏拒绝 | run/context/input、cluster application、attempt 前缀、真实 receipt、完整目录及首次选择均相互绑定；v5 的 11 类、v6 增加 A Captured/B 后的 13 类事实版本不串用，Legacy 不占业务版本。摘要自洽但父关系/时间/owner/代次矛盾仍拒绝，拒绝前后不修复数据。 | 先用正常接口生成事实，再对每类单独损坏并恢复原 guard 定义；公开 inspect/prepare 及重开拒绝，快照验证无写入。合法跨代作为对照；通用 run/concept/cache 与写后验证也必须覆盖新版本碰撞。 |

## 测试与完成边界

### 2026-09-13：真实旧v5事实迁移维护组

session89312的同一合批136项全部通过（3348过滤，编译7m39s/测试160.83s、50告警）；592项源码前后及当次当前、完整日志摘要一致。实际测试名称与前次133逐一比较，没有删减，只增加独立warm维护例和两个迁移例。此前34176仅因新增测试JSON宏索引语法编译失败；只增加三个括号对后，完整旧测试正文可由反向SHA核实未变，没有修改正常Rust/SQL/Cargo或放宽业务断言。

迁移例先通过真实公开prepare产生v5三次RPC、确认Retry/两份成功Response及两条完整BR159，再迁移并验证原对象定义、所有旧表行、原head/raw/目录及首次选择保持。故障例在预开reader的真实读锁下要求迁移COMMIT失败、完整回滚；释放后的fresh视图和真关闭重开仍为5，随后可以正常迁至6。迁移A行只说明旧Status缺原材料，不补造新时间、诊断或业务版本；新factory重放至Positions停止，保持零新增RPC/审计。这里不是迁移中途Retry续接或旧终结错误可恢复的证明。

本批只增加测试，正常消费者继续使用有效的86972编译证据，无需为文档或状态变化重跑。136项也不是52个推送Unit的完成数，更不是生产数据库迁移验收。

### 2026-09-13：固定参考缓存与同集合测量

[schema实现](../../src/push_foundation/intent_store/chain_post_close_schema.rs)现仅缓存编译进来的v1–v6固定DDL经原构造器生成的只读参考值，各版本惰性初始化且失败不缓存。每次仍检查实际连接的目录、版本、封存、Status材料关系及其他原有运行条件；不缓存“这个数据库已通过”的结论，也不保存真实连接或任务/权限/审计事实。主控反向去掉缓存新增代码后，整个schema文件的SHA精确等于改前版本，证明原构造正文和实际库校验未被改写。

| 本机验证 | 执行结果 | 测试执行时间（不含编译） |
| --- | --- | --- |
| 改前 session45613 | 原133全部通过，590项源码/日志摘要已核 | 191.99秒 |
| 缓存后 session72208 | 同133全部通过，591项源码/日志摘要已核 | 92.70秒 |
| 新维护例 session90700 | 1项通过，591项源码/日志摘要已核 | 0.88秒 |

前两次已逐个比较实际测试名称，133项集合完全相同；缓存后显式排除了新增维护例，再用独立命令运行它。编译分别为5m50s/3m21s，不将编译时间差归因于运行时缓存。上述为相同配置下的单次本机测量，不是生产延迟、规模基准或稳定提速比例；实际库目录、13类事实与完整审计历史的成本仍需后续量化。

[新增维护例](../../src/push_foundation/intent_store/chain_post_close_schema_cache_tests.rs)以真实完整v6验证成功为前置，在同一个库分别增加异名index/trigger后，公开verifier拒绝且不修复；真正关闭重开仍拒绝，另一个健康v6库仍通过。它不依赖私有缓存计数，也不代表所有事实损坏情形已验证。相关消费者Clippy session86972已于16:10:51Z退出0（3m33s），591项源码/日志摘要独立一致；lib202/monitor2告警与66904相比无新增/移除诊断类别，仅编译、不执行monitor。

所有数据库和文件均属测试自有目录，所有来源和时钟显式提供；网络只限合成本机 server，连接、请求及 shutdown/join 有界。读锁故障期间不读取数据库文件或额外开关同库文件描述符。不得读取真实 `.env`、生产数据库或调用真实消息渠道。

主控持单一 Cargo 队列，整个 Rust/SQL 输入冻结后才运行；保留实际命令、退出状态、日志和源码摘要。运行失败先记录实际断言，不为了转绿删除要求。完成后还需相关旧行为回归、消费者编译与完整 Task 2 的独立审查。

本片完成也不等于盘后完整恢复，更不等于全部 52 个推送单元交付。后续来源/模型、报告与逐目标发送、真实调度、强完成状态、切换及回滚继续属于总目标。
