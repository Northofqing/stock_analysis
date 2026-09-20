# W16 activation、部署认证与 owner fence 设计

日期：2026-09-08。状态：完整读取 `1c16380`、内部事务/原始准入投影 `10f7e03`、测试补充 `5a78dd6` 与Unix/FD真实观察及日历声明绑定 `9722979` 已通过限定验证/独立审查；B/C工程合同已定，生产身份/批准真实性、当前fence及owner执行仍未完成。没有认证部署或执行生产操作。配套计划：[W16 可执行计划](../plans/2026-09-08-push-foundation-w16-activation.md)；当前证据：[W16 实施结果](../../push-system/implementation-w16-results-2026-09-08.md)。

## 目标、依据与范围

交付完整 W16：认证部署与操作员、完整 manifest/journal 读写验证、逐 Unit generation CAS、全 Unit 当日晋级配额、legacy/new scheduler/producer/dispatcher/finalizer 的当前 fence，以及非原子 owner 切换、恢复、rollback。只读原始事实 reader 是其中一个切片，不构成 W16 完成。

权威是 `docs/push-system/push-system-wbs.v1.json` 的 W16：依赖 W06/W08/W11/W12，验收为“同事务验证全 Unit 当日 journal 和 generation；legacy/new 四类 actor 共同 fence”。不新增 W15 Ready 的循环前提，不含逐 Unit cutover。RFC `push-system-implementation-rfc.md` 610–648、753–762、993–1080、1130 起、1225 起和冻结 `push-system-foundation.v1.sql` 240–337 为具体合同；蓝图 §24.15、§24.18 约束受控重启、物理 owner 与生产批准。

开发授权覆盖实现与隔离验证；RFC 操作员合同明确 Codex 只准备证据与命令，不能成为生产批准者或执行者。本文不授予生产 DB、真实消息、部署切换权限，也不填造外部批准。Foundation 可以实现完整能力且默认关闭；52 Unit 的真实接管与六门禁证据仍逐 Unit 验收。

## 设计初稿基线事实与可复用 interface

下表记录设计初稿 `36a4be2` 时的可复用基础，不把当时缺口误报为当前状态。复用已记录的 W15 来源接线证据，不重新审计已闭合锁、codec、store；后续真实交付以实施结果为准。

| 当前真实位置 | 能复用的能力 | 当前不具备的能力 |
| --- | --- | --- |
| `src/monitor/push_job/catalog.rs`，`MachineCatalog::bundled/units/producers_for_unit` | 65 kind/102 producer/52 Unit 的精确注册与 owner 对应 | 部署配置、当前实际 owner 或批准 |
| `src/monitor/push_job/canonical.rs` | canonical tuple 的 domain + NUL + 排序 JSON、SHA 算法 | manifest/journal 专属字段合同和真实性 |
| `src/push_foundation/readiness_store_schema.rs::with_rollback_read_only` 与已审阅 ac8a28b reader 基础 | 同一只读事务、内嵌 schema 验证能力 | source authority、WAL 支持或写事务 |
| `src/push_foundation/intent_store.rs::BusinessIntentStore` | 稳定 intent、append-only transitions、lease/version | activation generation，不可把 lease_generation 替代它 |
| `src/push_foundation/reconciler.rs::reconcile_startup`、`StartupRecoveryReport` | 既存 intent 恢复及 scheduler barrier | 全进程物理 owner 协调 |
| `generic_transport.rs::GenericTransportAuthorityAdapter`、`business_finalizer.rs::FinalizerFence` | 现有运输/完成执行 seam | 当前 activation fence；现有 dispatch/lease token 不足 |
| `src/push_foundation/phase_scheduler.rs` | 纯调度评估 | 文件头明确没有 timer/I/O 接线 |
| `src/bin/monitor/main.rs::supervise_long_running_lifecycle` | 实际 monitor 生命周期落位 | W16 supervisor owner 协议，需新建 adapter |
| `src/auth/operator.rs::require_monitor_operator_auth` | opt-in PAM 密码认证流程 | 默认可跳过且只返回 `Result<()>`，不能据此认定已认证/已授权 |
| `src/calendar.rs::verified_a_share_trading_day` 等 | 已验证日历数据路径 | 请求自报日期不是认证的 UTC 配额区间 |

初稿基线时 `src/push_foundation/mod.rs` 没有 activation module；`1c16380` 已新增 raw activation 读取模块，但尚无完整 W16 operator control plane。`AuthenticatedOperatorRef` 是文字值合同，不是身份提供方认证结果。下述授权/执行 interface 仍为待实现设计，不能因原始读取入口已存在而推断已认证。

## Module 与信任根

采用一个 deep module `activation`，外部 interface 只暴露 `inspect(scope)`、`plan(request)`、`apply(request)`、`reconcile(command_ref)` 和受执行临界区保护的 `with_current_authority(actor, operation)`。内部拆为 facts/codec/store、authorization/deployment、owner runtime 三个实现簇；调用者不用自行组合 CAS、配额、owner 查询或选择 verifier。只读 interface 永不 provider/sink/transition。内部 seam 具有真实适配差异：SQLite 与隔离测试数据库、真实 supervisor 与可控进程 harness、平台认证与拒绝/过期测试身份。测试 adapter 只在测试编译下创建，不由生产配置字符串选择。

信任分四层，任一缺失即 Refused/CoreUnready：

1. 操作身份：由主机/服务身份提供方认证真实主体，再由受保护的 production allowlist 决定该主体可操作的 namespace、Unit、command（含独立 rollback 权限）、角色及有效期。在线批准精确绑定 command hash、Unit、generation、manifest hash、窗口、证据包 hash；不可复用旧批准。
2. 部署制品：批准的 release/build digest 必须与 supervisor 实际启动且持续持有的制品、Git commit、服务身份、配置/namespace 一致；实际打开的受保护文件及 owner/权限要验证，不能只验证包内自报 hash。防路径替换、旧进程、克隆自洽库和伪服务实例。部署身份与来源实例身份不能由请求指定为“可信”。
3. 来源绑定：沿 W15 接线提案，由认证部署准备流程将闭集 source-contract/解释器版本、source instance、受保护 locator、namespace、Unit/generation、实际 source binding package 字节绑定到 manifest 的 `source_contract_sha256`；来源 acquisition binding 精确联结 audit ID/record hash/采集上下文。旧 audit 缺这些事实不追认；跨库写入只接受 audit 与追加 binding 两者都存在。
4. 当前执行事实：完整 manifest、journal、实际 owner、gate 与部署集合一致才授予某次操作。已认证的读取结果不授权未来发送，执行临界区必须重验。

**needs-context A（外部配置）**：当前代码不能证明哪一个 production host/service identity、allowlist 管理者、部署制品签发/批准根、source locator 管理者和防回滚持久根已部署。推荐首版受保护的本机 supervisor/Unix 服务身份 + 强制交互 PAM 操作员 + root/服务管理员管理的 allowlist/部署批准存储，提供明确的所有者、权限、轮换/撤销与可信时钟策略；若生产使用服务身份，替换认证 adapter 并提供服务凭据验证。不能把环境变量用户名、PAM 可跳过的 Ok、任意 URI/SHA 或自由构造 `Verified...` 当替代方案。主控应确认实际部署平台和身份发行方；在此之前可完成接口、隔离实现及拒绝路径，真实认证接线不能报完成。

## 完整 manifest/journal 验证

schema 来源保持冻结 DDL，不新增第二状态机、不改两表或 triggers。Activation DB 的身份/namespace 从认证部署绑定中取得；不能因表无 namespace 列就混用不同业务环境。每次读取在同一事务确认真实 schema 与实际行，按 Unit 从首代验证到当前代，不能只读 MAX(generation)。完整性错误不归为空库，只有经过部署配置批准的初始化才允许 generation=1。

manifest 逐列严格读取 SQLite 存储类型并纳入 canonical：`unit_id,generation,previous_manifest_sha256,desired_state,physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at`；仅排除自身 `manifest_sha256`。所有 SHA 小写 64 hex、Git 40 hex，检查字符及字节长度/NUL，整数在非负 i64 内，generation ≥ 1。批准时间≤创建时间；窗口非空；Unit 必须存在 catalog。完整版本 hash 必须精确绑定当前/目标实物及批准证据，而不只检查格式。

journal 逐列纳入 canonical，排除自身 `canonical_sha256`：`event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,action,reason,window_start,window_end,evidence_sha256,rollback_target_sha256,previous_sha256,occurred_at`。稳定 event ID 用 RFC `PromotionV1(unit_id,generation)`；首代前驱为 NULL，后续前驱 hash 等于前事件 canonical；manifest 及 journal 都连续且无跳代。六种 action 必须匹配唯一合法边与 `activation.applied`；严格验证所有 FK 等式、窗口半开区间、actor=approved_by、approved_at≤occurred_at、rollback 目标同 Unit 的更早代及目标 state/owner。SQL 能拦住的检查应用仍需对读入事实重算，actor 等式不代替认证。

首代仅 Disabled；普通边仅 Disabled→Shadow→Active→Draining→Disabled。rollback 用新代指向兼容旧目标。journal 比 manifest 少一代为待协调，不能视为已执行、跳过或自动补成功。缺代、额外 journal、非法边、伪 hash、旧 schema/trigger 被替换均拒绝认证。

**B 已澄清（2026-09-08，尚待执行实现）**：原 Q13 的 shadow 指新路径，Q17 要求 Foundation 保留既有 owner；RFC 曾扩大为整个 Unit None，现已改为 ShadowActorPhysicalOwner。初始 Disabled/Shadow 的 manifest 保存经批准认证的实际 incumbent，legacy 在当前同一 fence 下继续原范围；新 shadow 无副作用权限。排空后的 Disabled/Shadow 保持关闭，Rollback 按精确旧目标与本次显式批准在新代恢复相应范围。准入从不可变已执行历史派生，不新增状态/权限库。完整矩阵、证据和代价见 [W16 合同裁决](../../push-system/activation-contract-decisions-2026-09-08.md)。该澄清解除文字矛盾，不证明实际 incumbent、旧 binary 的 fence 或认证 adapter 已实现；None 文本始终不授予 actor 权限。

读取编码裁决（2026-09-08）：核对现有冻结 Task2/RFC 与 canonical 实现后，稳定身份沿用 `PromotionV1`；新增内容 domain 为 `ActivationManifestV1` / `PromotionJournalV1`。三者均使用现有 canonical-v1：domain、单个 NUL、按列名键排序且无空白的 JSON object，文本按既有 canonical 转义，非负整数为 JSON 数字，可空列显式 null。manifest 含除自身 hash 外全部19列；journal 含除自身 canonical hash 外全部14列，包含稳定 event_id；`PromotionV1` 身份只含 generation 和 unit_id。独立 golden bytes 验证这些精确字段/编码，不新增 SQL schema version。成本是未来编码变化必须换 domain，不能同名改写历史。

T1 对 physical_owner 保留严格原始 TEXT，不解释执行准入；输出不是部署认证或 current fence。它可先完整验证全 Unit 历史并区分未登记、持久跟齐和末代待协调。缺失不是 Disabled，两个以上未执行代拒绝。当前 catalog 用于 Unit ID 注册关系，不据此宣称历史版本 SHA 等于当前已安装制品；历史到当前版本兼容和真实 owner 认证仍由后续任务交付。B 的准入投影由 T2/T4/T5 强制，不由 T1 raw 对象冒充完成。

## 事务、真实 owner 与受控重启

推荐保持蓝图“无热加载、受控重启”。认证批准包先在独立控制面持久保存，包含完整拟提交 manifest 字节与在线批准；它是批准事实，不是执行 journal。目标 binary 可预装，但默认所有新发生工作 gate 关闭。

1. `plan` 只读输出 before/投影 after；apply 重新认证、检查六类新鲜门禁、波次顺序、未决 ResolutionRequired/Uncertain、积压和双库一致性。未认证或 dry-run 到此停止，affected_rows=0，只有独立 control-plane audit envelope 可以被授权控制面接收。
2. supervisor 对精确 Unit/actor 集合 quiesce：关闭旧 actor 的新发生工作入口，等待已授予执行许可释放；旧进程若不支持共同 fence，则确认退出并撤销其 transport/业务写入口访问。仅 PID 或“发过停止信号”不够；拒绝无法确认存活状态的旧 binary。新的进程按批准制品启动为 paused，不能按日志激活。
3. 持 `BEGIN IMMEDIATE`：重新读取全 Unit 当前 journal/manifest、认证日历区间、逐 Unit expected generation 与 predecessor；若其他 Unit 有未协调 owner 操作，先隔离并协调，不能利用未记 journal 窃取名额。正常改变 owner 的晋级查询所有 Unit：`action IN ('Activate','Rollback') AND occurred_at >= start AND occurred_at < end` 任一命中即拒绝。内存锁不是配额权威。
4. 在同一事务中 INSERT 新 manifest；supervisor 将目标 owner 安装为已切换但 gate 关闭（无业务发送）。确认真实 owner 与目标制品/namespace/Unit 匹配，追加 journal，提交。此段外部调用需有期限；失败/超时关闭 gate，回滚 DB 并保留独立控制面操作事实。数据库事务不回滚已经发生的 owner 切换。
5. 新事务重查 manifest/journal 和实际 owner、当前批准与所有 gate；一致后才授予当前代执行许可。提交确认丢失禁止重试切换/重发；按稳定 command/event ID 重查，发现完全同一事件即幂等确认，冲突或缺失进入协调。

步骤 2 排空不持长时间数据库事务；步骤 3–4 为单写者短事务，且锁内的 owner 安装只操作已经预装并 paused 的实例。配额跨日时重新取得 authority 区间、窗口及批准；不能沿用请求发生日或本机日期。rollback 任意时刻可越过日名额限制，仍走新代 CAS 和同事务 journal，且当日后续 promote 会被它阻断。shadow/无 owner 变化部署不消耗晋级名额；冻结状态机没有 Active→Active 普通边，不能伪造这种 journal。若无 owner 变化的真实 release 也要求新 manifest，需明确使用合法现有边或另行规范设计。

**C 工程协议已裁定（实际 adapter 仍待）**：RFC 637 起先批准 manifest、1073 起同事务写 manifest+journal，蓝图要求重启。采用“先外部持久批准包，事务内插入 manifest + 有界 paused owner 确认 + journal”的解释；批准包不是先提交的 activation 表行或成功 journal。详见 [合同裁决决定三](../../push-system/activation-contract-decisions-2026-09-08.md)。真实 supervisor 跨进程持久协调存储/恢复 interface、生产认证根仍待接线，不能因事务测试通过就宣称真实 owner 已切换。冻结 DDL 不变。

## 当前 fence 与状态职责

共同 token 精确绑定 `(unit_id,generation,manifest_sha256,physical_owner)`，并核对认证部署实例、namespace、action capability 和当前 gate。不可缓存后无限使用。`with_current_authority` 取得执行许可与 owner 撤销共享的线性化点，直到对应持久副作用/发送完成才释放；owner 切换必须等待许可结束。跨进程必须依赖可观测 supervisor/共享协调机制，不能只有当前进程 Mutex。持久 lease generation 仍用于 intent 竞争，与 activation generation 分别验证。

| actor（legacy/new 都适用） | 必须重验的操作 | 失效时行为 |
| --- | --- | --- |
| Scheduler | 产生/登记新 occurrence 之前 | 不创建工作，不以重启改 occurrence ID |
| Producer | 外部采集/prepare 及创建 intent 之前 | 停止新增；shadow 仅共享已采集 facts 的纯比较 |
| Dispatcher | 真实 transport attempt 紧前及许可存续期间 | 拒绝旧 owner/旧代；已发 Uncertain 交原 decision 恢复 |
| Finalizer | business completion/cursor 事务紧前 | 只允许当前恢复职责、原稳定 intent/terminal binding，旧 owner 不重写完成 |

Active 仅 manifest owner 获得新发生工作资格。shadow 执行路径无发送/写业务/推进 cursor 权限，用计数拒绝 capability 证明零副作用，可多 Unit 并行；legacy 的准入按 B 的不可变历史投影与实际认证取得。Foundation 初始 Disabled 关闭新框架的 scheduler/producer/dispatch，保留证据确认的 incumbent 原范围，但不绕过当前 fence。正式执行 Draining→Disabled 则关闭该 Unit 新工作、保留持久 pending、Accepted、Uncertain 与其恢复；其后进入 Shadow 也不自动开启 legacy。Draining 禁止新增 occurrence 和 prepare，但保留 authority 查询、finalizer、reconciler、隔离。恢复职责单独按当前执行 fence 委派，不用旧 owner token 续权，不以“没有新 owner”删除责任。Disable 必须有排空证据；不可把未决状态当终结来通过转换。

W16 基础阶段把共用 actor seam 与 monitor/CLI 入口接好且保持关闭默认；Unit 特有 legacy adapter 未覆盖时，该 Unit 的 promotion eligibility 必须拒绝，不能让测试一个 Unit 的 fence 冒充 52 Unit 实际接线。每个后续 Unit cutover 提供完整四类 actor 映射及无旁路证据。

## 中断协调与 rollback

| 故障点 | 可见事实 | 允许的恢复 |
| --- | --- | --- |
| 批准后、quiesce 前 | 控制面批准存在，旧 manifest/journal/owner 一致 | 重新认证再计划；不自动启用新代 |
| 旧 owner 已停、DB 未写 | 旧已执行代仍在；实际 owner 停止 | 保持不就绪，精确核验后恢复旧 owner 或继续获批操作 |
| 新 owner paused、事务未提交/回滚 | DB 旧代，实际 owner 新制品 | 关闭所有新工作；审计并恢复旧 owner 或重新执行同一批准的 CAS，不补造执行事实 |
| 提交成功但确认丢失 | 新 manifest+journal 已存在，owner 可能 paused | 新连接精确重查，按相同事件确认；不得再推进一代 |
| journal 新代、owner 不匹配或 owner 查询超时 | 持久成功事实与实物不一致 | Core/受影响 Unit 不就绪，先隔离；认证 reconcile 修复到已执行目标，重新验证后开门 |
| 孤立 manifest/进程重启/外部批准过期 | 未执行代或协调状态未知 | 禁止跳代；重新授权协调，日志不触发 activation |

rollback 始终生成 N+1，不降低 generation；目标为同 Unit 兼容历史 manifest。验证当前业务及 durable schema、binary 支持范围、模板/原始 bytes、source-contract 解释器与 N/N−1 兼容矩阵，拒绝跨 Unit、破坏性降 schema 或没有证明的旧 binary。N−1 必须实际支持共同 fence 才能取得执行许可；不支持的 binary 只能由已验证 supervisor 保持暂停/禁止启动，数据库新 generation 本身不会约束旧代码。保留 AcceptedPending/Uncertain/ResolutionRequired、原 decision/occurrence/lease 与精确字节；外部 Accepted 不撤销，Uncertain 不盲重发。rollback 新 owner 接管恢复职责，旧 binary 即使重启也不能绕过共同 fence。

## W15 的全范围最小兼容策略

当前 `ReadinessSnapshotContext` 只有一份 `activation_generation/manifest_sha256/build_commit`；`ReadinessStreamId::for_snapshot` 用这些 scalar 分流。Core 评估的 producer 集却可跨 Unit。因此仅改调用者选最大代、某一 Unit、hash 填入 manifest 字段或只评一个 Unit 均不满足 RFC。

当前已审阅基线 595f605 是 `OperationalReadinessSnapshot/v2` 与 `OperationalReadinessMaterial/v2`；decoder 要求 schema_version=2，legacy v1 按当前策略拒绝。保留现有 v2 候选 bytes/哈希/字段语义，不把部署集合塞进同名 v2 domain。[部署集合合同](../../push-system/activation-deployment-set-contract-2026-09-08.md)已确定新 `OperationalReadinessSnapshot/v3`、`OperationalReadinessMaterial/v3` 和独立 `ActivationDeploymentSet/v1` domain，以及集合的精确编码。版本名已确定不表示消费者已支持；v3具体wire、store/probe消费和跨stream恢复仍待实现验收。

集合以 UnitId 稳定排序，精确成员包括 `(unit_id,generation,manifest_sha256,journal_event_id,journal_sha256,physical_owner,build_commit,build_sha256,source_binding_sha256)`；绑定 namespace、catalog hash、认证配置的启用 producer/Unit 集、仍有恢复责任的 Unit、日历版本及共享依赖版本，独立派生 `deployment_set_sha256`。该 hash 不冒充任意单 Unit manifest。

T6A原始候选集合已实施至`e0cdd0d`并通过限定复核，具体字段、未登记null及Core6文本排序按[集合合同](../../push-system/activation-deployment-set-contract-2026-09-08.md)。它比较来源/配置声明，不把声明认证为真；下面的真实认证、跨库消费、v3和执行许可要求没有因此完成。

加载时从同一 activation DB 事务读取全部登记 Unit 的状态，认证启用集合与 catalog 的闭合；Inactive/Disabled/Shadow 必须显式登记状态和来源，不把缺行当批准 Disabled。Core 覆盖全启用 Unit 及共享前提；未启用但有 persisted pending 的 Unit 加入恢复覆盖，不因此启用新工作。每个 Producer/Occurrence join 到所属 Unit 的精确 entry。跨 activation/source/readiness 库不宣称原子快照：读前/提交后比较认证集合和来源版本，任何相关 generation/配置漂移拒绝认证该 snapshot 并重评；执行仍需当前 fence。

新增集合 stream identity 绑定完整集合与 scope，使用新 stream domain/version，不覆写现有 scalar stream identity；snapshot/material 和 recovery record 加载按明确版本 dispatch，现有 v2 保持原候选语义，建议 v3 走集合语义，未知版本和 legacy snapshot v1 继续拒绝。recovery 明确前后集合差异及受认证恢复证据，跨 stream/domain 不隐式续接或升级历史。现有冻结 recovery event/domain/schema 保持原样，优先仅对其引用 snapshot/material 的解析做显式版本分派；只有证明事件语义或存储结构确实无法承载时，才提出独立版本修订，不因 snapshot 升级自动重写 event/schema。

新 probe/health/CLI 共享同一集合版（建议 v3）snapshot，输出集合 hash 和逐 Unit 代，不输出伪全局 generation。现有 v2 可以按既有候选合同读取，不能自动升级为全局认证 Ready；本文不承诺 v1 可 inspect。store 优先保留当前 schema、历史 bytes/hash 和明确版本 codec dispatch；变更 schema 必须另有实际不兼容证据与审查。不得改冻结 Foundation DDL 来塞 readiness 列。

**D（版本选择已定，消费合同/实现尚待）**：新增[部署集合合同](../../push-system/activation-deployment-set-contract-2026-09-08.md)确定独立`ActivationDeploymentSet/v1`、后续snapshot/material v3及`OperationalReadinessStream/v2`，固定集合完整字段；旧v2候选与stream v1、legacy snapshot v1拒绝保持不变。具体v3 wire、recovery跨stream显式衔接、RFC输出和新蓝图视图仍待T6接线，不改原冻结输入。W15 snapshot/recovery/probe/stream 与认证接线属于实际缺口，任务并行推进不代表 W15 Ready 已成立。reader 现只接受 rollback 模式，生产 BR159 默认 WAL；推荐先给 activation 存储明确 rollback 配置，BR159 单独交付 WAL 一致读或来源认证导出（不得复制主文件遗漏 WAL），由来源工作项落地。无法认证的 source 持续不就绪。

## 操作员 wire 与拒绝语义

实现 RFC 1130 起 request/response 全字段：稳定 command_id、typed target、expected_version/generation、显式 dry_run、认证主体引用、ReasonCode、typed evidence refs、requested_at；输出 Inspected/Planned/Applied/Refused、精确 before/after refs、affected_rows、成功 mutation ref 或 NULL、独立 operator audit ref、拒绝原因、canonical snapshot hash。dry-run/refusal 不写 activation/business/durable DB 或 promotion journal，也不切 owner/调用 provider/LLM/sink/order。

W16 实现 inspect/promote/rollback 及 activation 协调，既存 intent 的 reconcile/resolve-uncertain 继续使用其原 authority，不扩展成万能 SQL 编辑器。所有请求经应用校验，禁止直接编辑 SQLite。六门禁逐 Unit/build/manifest/generation 绑定新鲜证据，检查 wave rank、在线批准、source/presentation/policy、积压/Uncertain/双库一致性；证据缺失只能拒绝，不由 W16 自行宣称通过。

**身份关系与条件性 needs-context E**：SQL `journal.actor=manifest.approved_by` 已冻结。RFC 1170–1184 的 DualControl 要求 preparer 与 approver 为不同认证主体，并不要求 approver 与 executor 分离。因此采用独立 preparer、真实 approver 批准并执行，在独立控制面完整保存三个角色绑定即可满足现有合同，无需改冻结 DDL，也不据此阻断开发。只有组织额外要求 approver 与 executor 分离时，该外部策略才列 needs-context，主控须裁决版本化规范兼容方式；不得把执行者伪标成批准者或用服务账号混淆身份。SingleControl 可以同一真实主体批准并执行；紧急 rollback 要显式权限和独立审计。

## 可证明的完成条件

配套计划每个任务提供独立反例、精确文件 ownership 与验证命令。测试仅临时目录、Test namespace、合成库与本地可控进程，不读取真实 `.env`/`data/**`、不调用 provider/真实 PAM/消息渠道，不执行 production approve/apply。完整 W16 必须同时通过两进程 CAS/配额竞争、四类旧/新 actor 撤权、owner 切换每个故障点、全 Unit context 漂移、rollback 原 pending 保留，以及所选真实平台 adapter 的隔离验证。外部信任根尚未配置、Shadow/legacy 准入尚未实现、仅 facts reader 通过、未完成四类 actor seam 或新集合版（建议 v3）全范围接线时，报告相应未完成项，不能升级为 W16 Ready/生产认证。版本验收须保持当前 v2 候选语义和 legacy v1 拒绝策略。
