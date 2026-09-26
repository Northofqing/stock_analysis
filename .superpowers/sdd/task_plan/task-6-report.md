# Task 6 — 历史构建证据与当前连接资格分离

基线：`dd0f9e644cdc714c4d6018da928028c655218496`；开始时源码工作树干净。依据 Task6 plan、预检、authoritative audit D11/D12 和当前源码；不使用旧预检行号替代当前证据。不连接外部服务、不读取生产数据库、不部署或自动迁移。

## Fix round 1 — review I01 / M01（基线 2438ca76）

本轮只处理 `task-6-review.md` 的 I01 和 M01，不混 Task7，不访问生产、外部服务或执行部署。下文原始实现阶段的测试记录保留，本节为本轮修复证据。

结果：I01/M01 修复完成并交复审；核心36/36、最后接口回归43/43、非测试库检查通过。尚未部署；不把本轮源码验证等同于 Task10 生产验收。

### I01 最小修复与兼容边界

- 新增 `grpc_client::external_decoder::ExternalDecoder` 的代码所有、封闭 descriptor→decoder 目录。未知 descriptor 拒绝；已知 descriptor 仅表示可解码，当前/历史 build policy 和 connection receipt 仍独立校验，不从响应或环境学习可信版本。
- V1–V3 control/data/status 固定使用归档 A decoder。V4 依据严格绑定的 connection policy/descriptor 选择当前 decoder；当前 request 与旧 request 分别检查各自 canonical 合同。旧 A GlobalNews plan/request/id/ordinal/backoff/deadline 不改写；A→B continuation 只支持明确兼容、字节相同的请求形状，并在发业务前用 B decoder 验证，不承诺任意不兼容的未来升级。
- 控制传输先保存实际 protobuf bytes，再映射到领域类型；避免先用 A 类型重编码而抹去 B 合法字段。Capabilities catalog、Health build qualification、wire/status/data replay 使用相同版本选择。data V4 增加可选 connection identity（旧版本省略，旧 bytes 不变），写入及恢复绑定原 effect-link 的 epoch/policy/descriptor；错误 epoch、缺失身份和把 B data 降级成 A/V3 拒绝。
- 测试 B 不是假摘要：build.rs 从冻结 A 明确生成独立的 additive B proto，HealthResponse / CapabilitiesResponse / QueryResponse / ErrorDetail 各声明 tag127；独立编译真实 descriptor，SHA256=`95aef6923dba1f688a911bca9a1b81c3c2c3c6e423965fd96026e37032d85fff`。真实 localhost tonic+mTLS 服务实际发送这些字段，B policy 的 contract digest 也不同。生成的 B 消息/信任条目仅 `cfg(test)` 可用；生产支持新发布仍需显式代码/可信目录发布与 Task10 运维资格。
- 原 A metadata/proto、client-bundle、v11/v12/v14/v15 DDL/seal 本轮均无修改。无需新 schema/migration；旧 control/schema/canonical/raw golden 持续验证，不通过改旧事实“修复”历史。

### M01 终态重开

共享 retry 场景在 A→A 和 A→B 成功终态后关闭数据库、真正重开、恢复 lease 并再次 drive。比较所有 `chain_post_close_*` 不可变事实与 `data_acquisition_audit` 行（包含 bytes/hash；允许正常 lease/run 元数据变化），以及 Health/Capabilities/data 请求、TCP 接收数量，要求完全不变。此项是缺失回归的补充：原生产 early-return 已正确，未伪造行为 RED，也未改终态逻辑。

### 本轮验证记录

1. 真正 RED，session40727：`cargo test --lib task6_current_control_v4_uses_recorded_b_policy_not_legacy_a -- --nocapture`。编译3m16s；0/1，B 真实 descriptor + 合法新字段的 V4 project 返回 `SchemaRejected`，复现 I01。
2. GREEN 接线初试，session74086：同一单目标；6个编译错误（http 路径、Sync 约束、两处旧测试 initializer 缺 optional identity），非行为 RED。修复后 session64049 的3目标命令仍报 fixture 缺 `Message` trait 导入3处；修复导入，不更改业务断言。
3. 首次 GREEN，session41054：`cargo test --lib -- task6_current_control_v4_uses_recorded_b_policy_not_legacy_a task6_historical_build_a_v3_reopens_under_client_b_without_changing_bytes task6_durable_b_legacy_qualified --nocapture --test-threads=1`。3/3，编译3m16s、测试16.70s。证明 A 历史、B V4、真实 B 新资格/Capabilities/data 写入与重开。
4. 核心限定回归：`cargo test --lib -- macro_codec::tests task6_durable_b_ task6_connection_ single_user_external_macro_confirmed_data_retry --nocapture --test-threads=1`。session81699 中21条 codec通过，但15条网络用例在 localhost bind 时被沙箱拒绝，未进入业务，不能计为业务 RED/GREEN；按权限规则原命令提权重跑 session9282，36/36 GREEN，编译3m16s、测试185.70s。覆盖 B descriptor 升级/拒绝/Unknown、物理代次、持久 journal/effect-link、旧 A bytes、V4 status/carrier、原 retry 与两种终态重开。
5. 最后接口回归 session28354：`cargo test --lib -- new_external_data_result_binds_request_and_descriptor_identity external_control_attempt_tests external_native_control_tests external_query_transport_tests grpc_client::errors::tests --nocapture --test-threads=1`。43/43 GREEN，编译3m14s、测试5.29s；包含最后加强的 V4 inner success 与反降级/epoch 断言，以及实际 control/native/status/provider/carrier 回归。过滤项中没有独立 `external_query_transport_tests` 命名测试；不按过滤词冒称额外测试数，原始数据 wire/source11 已由前一组 codec 和实际 transport 场景验证。
6. 最终非测试 `cargo check --lib`（session12466）通过，1m14s，既有 warning 非零但无编译错误。`git diff --check` 已通过；`git diff --quiet HEAD -- contracts/external_v1_history client-bundle src/push_foundation/intent_store/chain_post_close.v11.sql src/push_foundation/intent_store/chain_post_close.v12.sql src/push_foundation/intent_store/chain_post_close.v14.sql src/push_foundation/intent_store/chain_post_close.v15.sql` 返回0，证实冻结资产未变。停止验证，不追加无关全量/check/build/clippy。

### 改动面 / 自审

`build.rs` 仅生成独立测试 B 合同；`grpc_client/{external_decoder,build_identity,client,errors,external_control_attempt,external_query_transport,historical_external,unary_attempt,mod}` 负责闭合 current/frozen decoder 和原始字节传输；`chain_post_close_macro{,_codec,_connection,_live,_native,_recovery}` 负责 V4 data 与已存在 effect-link 绑定。两组 fixture/codec 测试覆盖新增合同和终态恢复。未改变业务源、重试策略、旧报告、生产参数或数据库对象；没有新发布授权旁路。

未覆盖：真实上游未来 B 发布及部署、生产迁移重资格、已排队并发 RPC 的完整断网混沌矩阵。当前证据证明 one-dial 与既有双 mTLS 同 URI 切换行为，不夸大为所有网络时序。保留 Task10 运维验收边界。

## 原实现阶段最终状态

Task6 源码实现完成，交独立review；未上线。最终专项18/18、旧布局兼容/重试13/13、最终v15 seal/迁移2/2通过；非测试`cargo check --lib`通过；暂存`git diff --cached --check`通过。先前单项失败均已修复并在相关后续目标复验，不把有失败的合并命令描述成全绿；完整RED/GREEN及时间见下文。保留真实SQLite commit contention/Unknown、旧raw bytes/hash、原request/backoff/deadline断言。没有跑无关全量，也没有执行release/生产迁移/上线。

## 交付面与兼容边界

- 历史真实性：归档公开A metadata与proto，message-only历史decoder读取V1–V3；不创建RPC client。显式未知policy/descriptor/原请求绑定拒绝。当前资格使用当前编译可信policy，不能由响应/环境自行注册。V4才有新connection identity；旧codec未补字段、旧DDL/seal/hash未修改。
- 物理连接：one-dial connector保留tonic TLS/mTLS；同Channel/clone第二次dial在TCP前拒绝并撤销代次。Health验证只提升内存活跃generation，序列化identity仅作证据。普通query、授权unary、Capabilities、事件/监听/watchlist入口均经同门。
- 持久资格：layout15新增资格facts，Begin/Result/EffectLink进入原run事务、CAS与审计链；首Health可复用同代原请求，重开必须独立Health及当前Capabilities。原业务Unknown、资格Unknown都先拒绝；新资格不改原ordinal/backoff/deadline。
- 旧single-source续跑：仅原v11完整验证的GlobalNews scope；共用现有runner/Live，不补LocalRoute、不新增EconomicCalendar或第二来源，不将source final伪装成完整StageFinal。新的宏观/模型factory显式要求15，旧生产writer/driver不可达。
- 后继安装：仅提供真实显式v14→15 maintenance migration；普通startup/reopen不安装。运行前仍需Task10备份/lease/资格重发/owner切换，旧layout新external effect失败封闭，不把“源码已可迁移”写成“生产已运行”。

改动文件归组：`build.rs`/Cargo只增加历史message codegen与现有hyper-util显式依赖；`grpc_client/{build_identity,connection_qualification,historical_external,client,external_control_attempt,macro_attempt,external_query_transport,errors}`负责历史/transport；`chain_post_close{,.v15.sql,_schema*,_macro*}`负责后继catalog、qualification、runner适配与恢复；原board/cluster/concept/positions/dragon_tiger/models校验只扩展已知layout15，不改变对应业务。限定fixture/回归随新的显式资格合同更新。`contracts/external_v1_history/*`是冻结历史材料。

自审关注：无生产数据库访问/迁移/外网RPC/release/activation；真实同URI强制断流在transport层双mTLS服务验证，durable层覆盖重开A/A、A/B、Rejected、Unknown、重试/提交失败，不声称穷举每个网络断点的混沌测试。真实上游B发布的信任目录/descriptor、生产迁移资格、切换窗口与监控运行仍须Task10验收。独立review尚未执行。旧fixture专用append接口仅cfg(test)，不接受transport/session，不授予发送权限。

## 静态复核 / 最终 Interface 与主 owner 裁定

- 当前公开 bundle 为 `2026-09-17.1`（`client-bundle/bundle-metadata.json:2`）。README 要求 Health 四字段逐项匹配，不能由首次响应建立信任。历史证据需要保留该发布 metadata 与明确 descriptor 的冻结可信目录；当前 policy 与历史目录分离。
- D12 确认：`VerifiedBuildIdentity::validate` 调用当前编译 pin，Health 历史 project 又调用当前资格函数（`chain_post_close_macro_codec.rs:860`、`:1063`）。新的历史 Interface 只返回无发送能力的证据验证结果；严格验证原 bytes/hash/request/method/profile/受支持 descriptor/发布 identity。未知历史 policy/descriptor 失败封闭，不补造旧 V1/V2 的 connection receipt。
- D11 确认：prepared `connect_once` 当前只设置 route，`AuthorizedCapabilitiesAttempt` 和 macro prepared 可以 cold connect；v11 与 full driver 重开均没有原连接。新 Interface 是“未资格 session → 显式 Health qualification effect → 同代 qualified session”，所有 Capabilities/data/event 发送入口共用 gate；历史 Ready 不提供新 effect 权限。
- 运输 Interface：tonic 0.14.6 的 `Endpoint::connect_with_connector` 仍通过 `self.connector` 包装既有 TLS（本地一手源码 endpoint.rs:522、:586），可以在内部使用 one-dial connector；同一 Channel/clone 的第二次物理 dial 必须永久拒绝并撤销旧代资格。显式 owner 重建新的 epoch，并记录独立 Health 后才能重试原本允许的业务。URI/TLS/持久 epoch 字符串均不能复活活跃权限。
- **主 owner 已确认后继 layout=15**：冻结 v11/v12/v14 DDL、codec/hash/fixture；layout15 仅追加 qualification begin/result 与 effect-link 所需最小对象，沿用 run lease/version/CAS/审计链及 exact catalog/migration owner。旧布局只允许历史验证；新的 external effect 在显式迁移至 layout15、确认 exact catalog 和当前连接资格前拒绝。不得夹带 Task7–9 schema 或普通 startup 自动扩表。
- qualification 事实绑定：run/intent、原 deadline、独立 request ID/Health 原 wire/status、endpoint/authority、物理 epoch、可信 policy digest/descriptor/实际四字段、Qualified/Rejected/Unknown 与 predecessor。业务 begin/result 绑定该 receipt；数据库重开只能验证它，不能恢复 session。原 Unknown 先阻断，不以新资格顺便重发。
- 成本：断线后增加资格往返/可能暂时不可用；宁可拒绝业务，也不允许旧 Channel 自动落到新 build。真实 A/B 上游部署验收仍属于后续运维，本地双服务 fixture 只证明客户端行为。

- 状态机接线裁定补充：主 owner 确认 layout15 显式 journaled Qualification step（begin/result/link）；禁止 low-level execute/async future 借 store 或隐藏 Health。Unknown gate 最先执行。首次原 Health 只有在同物理 epoch、同 policy、同持久 receipt 可证明时可兼作资格。断线后不在同次 drive 自动重拨；未确认业务保持 Unknown，已确认 retryable/尚未 begin 业务可在下一次安全 drive 新资格后延续原 ordinal/backoff/deadline。

### layout15 后继事实与调用接线（本节为施工边界，不表示已完成）

- v11 continuation 裁定：主 owner 明确要求可续真正旧single-source GlobalNews，不能将缺LocalRoute的v11降成纯历史，也不能伪造LocalRoute/双源。layout15增加严格legacy-v11 continuation owner，共用同一资格journal；仅接受完整验证的原plan/version、endpoint/authority/descriptor/request identity，只续原GlobalNews并允许其原source final。v15新guard仅对此路径开口，普通旧writer仍拒绝；EconomicCalendar始终0 RPC。Unknown、缺失/冲突route、未知格式和资格不符均typed fail-closed。成本是保留一个窄legacy continuation Adapter和v11 A→B专项矩阵；不复制第二套资格/重试规则，不修改旧v11/v14 SQL/bytes。
- v11最小接线：复用现有runner的collect与同一Durable/Live资格路径，显式只调度Gateway(1)，不创建其余query/LocalRoute或完整StageFinal。恢复视图来自原single-source plan/attempt/source事实；结果需明确为LegacySourceConfirmed，不能用空字符串冒充完整Macro报告。旧格式data/source append由layout15资格link和单源guard授权，原v11普通owner不恢复写权限。

- `chain_post_close_macro_connection_facts` 是原 run/CAS 链上的追加式事实，不是新队列/第二业务账本。闭集 kinds 为 HealthBegin/HealthResult/CapabilitiesBegin/CapabilitiesResult/EffectLink；原 effect 的 request/ordinal/backoff/deadline 保持。单 epoch 的各控制事实唯一，EffectLink 指向紧接的原 effect begin，必须在同事务写入。
- 首次 Health：先分配 one-dial generation，再以同一个已冻结原请求同时绑定 qualification Begin 与原 Health Begin；响应时一起冻结 result 与 receipt。不能等到 async connect 后再凭 URI 补 epoch。
- 重开：原 Unknown/final 先检查；只有原 Ready 且业务允许继续时才追加新 qualification Health（不同 request ID）。旧 Capabilities 已 Ready 时必须另追加当前连接的 Capabilities 资格事实，不能使用旧 A capability 作为 B runtime 事实；这些新控制不覆盖旧 episode。
- 重资格Interface收口：runner仍使用现有Health/Capabilities步骤；durable Adapter在内存connection缺失时将待执行业务的route投影为NeedsHealth。独立qualification ticket与原control ticket分型；同步begin/record仍由同一Live与CAS管理，OwnedAttempt future只拥有transport/ticket。独立begin无result进入MacroRecovery的Unknown集合，与原业务Unknown一起在open最先拦截。并非重置原readiness episode、重试ordinal或deadline。
- 运输 client 的 planned/current identity 只提供可审计材料，不构成可复活的发送 token。业务 admit 必须同时持有活跃 one-dial client 和同 epoch/当前 policy 的已确认持久 receipt，先检查后写 effect link/begin，poll 再检查。重开只恢复证据、不恢复 session。
- V4控制codec（layout15新事实）：显式connection identity包含epoch/policy/descriptor；Health原响应与可信recorded policy严格比较，Capabilities的verified build来自同epoch已确认Health。恢复逐项核对qualification事实、原control或独立control结果及effect link；V1–V3的optional新字段为None且不序列化，不能给旧bytes补代次。测试B信任只来自cfg(test)编译固定目录，不改变实际A session的current pin，不接受response/env自注册。
- B持久矩阵的测试seam：既有v11 checkpoint helper允许显式fixture bundle路径，以同URI TCP switch生成原A记录；同一driver内部允许测试注入已正常prepare的External endpoint，生产仍由GrpcSource解析。Unknown/final gate早于route准备，原endpoint/profile/authority/request校验不变。不是第二套driver、retry engine或绕过资格token；B独立mTLS服务返回固定B身份，原历史A response/bytes仍直接核对。
- 恢复 reader 必须校验新增事实 bytes/hash、run/lease、原 plan/deadline、显式可信 policy、前驱、控制请求/响应及 effect link。新事实进入全局 run fact 唯一性和新结果完整性；旧 final 的输入字节不变。新增 qualification 未确认时以 Unknown 阻断，不以失败或重试回执替代。

## TDD 与验证记录（历史施工日志，最终结果优先）

- 最终seal收口session52045：`cargo test --lib -- bundled_v15_catalog_is_sealed task6_layout15_requires_explicit_migration --nocapture --test-threads=1`，GREEN2/2，编译3m13s、测试1.29s。最终新v15 SHA与262个精确对象、显式安装/重开/旧seals/shadow拒绝一致。之后只追加报告与交接，不再改源码。
- 最终专项session67861：`cargo test --lib task6_ -- --nocapture --test-threads=1`，GREEN18/18，复用编译0.68s、测试147.39s。包含实际同URI独立mTLS服务断流/拒绝自动重连、当前B独立资格及A历史读取、显式layout15/旧seal/shadow拒绝、首连journal Begin/Result/Link/篡改、旧Ready真重开Health/Capabilities、A→B成功/拒绝/资格Unknown/原business Unknown/confirmed retry和真正旧v11单源continuation。
- 最终非测试编译session97778：`cargo check --lib` GREEN（1m12s），确认旧writer/driver与离线历史fixture的cfg(test)隔离不会掩盖生产依赖缺失。未运行release/部署。
- 暂存后`git diff --cached --check`首次发现新建v15 SQL尾部多空行（此前untracked不出现在普通diff）。移除空行并同步尚未发布的v15 bundle seal，最终SHA256=`16a2f421398f6987e724c775ab550227d1e7d87827faeaefaa61e9f0a4bfd7e6`；旧11/12/14 DDL/metadata/hash完全不变。SQLite对象定义/262对象集合未改；仅追加`bundled_v15_catalog_is_sealed`与`task6_layout15_requires_explicit_migration`限定复核，不重复无关测试。暂存diff-check现已通过。
- session84412：`cargo test --lib v12_migration_tests -- --nocapture --test-threads=1`，复用编译1.00s、测试134.67s，9/11。旧mixed确实在`Live.begin_data(Gateway2)`因layout12缺当前资格拒绝；采用上述离线fixture seam，且先断言生产entry仍拒绝。另一个旧测试把14当未知future，而当前已识别14/15，故实际SchemaRejected；将伪造未来探针推进16，不更改旧DDL/hash或放松未知拒绝。真实旧迁移保存、commit contention、metadata/half-install/损坏9项通过。
- session19345限定验证：`cargo test --lib -- v12_migration_tests task6_connection_journal_data single_user_external_macro_confirmed_data_retry --nocapture --test-threads=1`。同时保留原A→A retry用例的成功raw golden（含zero-length source11尾字段）与native golden，共享A/B场景不丢弃这个历史兼容断言；只改变独立A successor fixture的真实wire编码选项。
- session19345最终GREEN 13/13，编译3m13s、测试170.52s。覆盖11项v12迁移/拒绝（含离线mixed原bytes/旧audit不变、普通begin拒绝和0新增网络）、资格immutable/tamper、原A→A retry的1000ms剩余backoff/ordinal2/deadline/zero-length wire与native golden。私有begin只新增受cfg(test)闭集variant控制的精确历史构造路径；随后限定`task6_`矩阵复核生产begin相同分支，并做非cfg(test) lib check确认测试入口不可进入owner binary。
- session76302：`cargo test --lib -- control_unknown_commit_tests single_user_external_macro_confirmed_data_retry single_user_external_macro_valid_ single_user_external_macro_real_v2_tamper single_user_external_macro_historical_ single_user_external_macro_unpublished_provider --nocapture --test-threads=1`，编译3m14s、测试206.61s，14/15。6项真实Begin/Result commit失败及receipt取消全GREEN，A→A retry、历史provider/valid-wire通过。唯一失败是共用first-source历史生成器真重开仍调用旧cold路径；主owner确认此helper必须保留layout11以服务7处历史迁移/篡改测试，改为显式新Health→同代qualified client→test-only旧事实append，不恢复生产旧driver。
- 新增资格事实的读侧边界补充到已有data-link测试：实际UPDATE/DELETE被不可变trigger拒绝；离线删除update trigger、篡改duplicated epoch列、恢复精确原trigger后，真重开reader必须拒绝。不伪称此加强断言是此前没有实现的行为RED；生产reader本已验证列/bytes/链的一致性。
- 后续限定session54052：原first-source共用helper、历史V2篡改、v12迁移/拒绝/损坏目标及data-link加强断言。禁止并发Cargo。本轮新增helper只用于历史fixture；实际v11 A→A/A→B continuation仍由layout15的同一driver完成。
- session54052结果8/9（编译3m14s、测试98.27s）：共用first-source、V2篡改及6项corruption通过；资格列篡改在`single_user_local_chain_post_close` owner创建阶段已经SchemaRejected，测试误unwrap，调整为核对这个更早的真实拒绝边界。此命令中的两个schema文件名不是模块过滤词，实际上未运行v12 migration；用真实模块`v12_migration_tests`的session84412补覆盖，不将过滤0项记为通过。
- 历史mixed fixture边界：主owner批准仅cfg(test) `begin_historical_v12_data_fixture`，只允许layout12+冻结完整旧Ready的离线旧格式构造，无新连接/RPC、无发送权限；生产begin/driver不可引用。它保留旧RawResultV2+nativeDataResult混合reader覆盖；不能通过迁15后给原v11单源plan新增Gateway2来伪造续跑授权。
- session77362：修正wire + Retry A→B + 原v12三目标，4/5（编译3m17s、测试63.21s）。wire与两条v12 Unknown、Retry A→B通过；唯一失败为迁测试时把既有LocalUnavailable报告预期误改retired，恢复原`no_verified_batch`，不改生产。Retry此前失败来自fixture每poll做全库read校验耗尽真实12秒monotonic剩余预算；改成服务已返Status后才读checkpoint，原15秒deadline/生产代码不变。
- session15039：原Health重开、控制历史、六项commit/receipt故障、v12完整provider trace，19项15/19（编译3m16s、测试215.98s）。Health重开、11项旧控制历史、两项receipt Unknown、完整v12 trace/report均通过；四个commit用例旧错误实现字符串/因果断言失败，新的typed AuthorityRejected/ResultUnconfirmed已经正确。更新为v15明确错误合同，并保留SHARED SQLite reader实际commit失败、原请求/row不变、Unknown真重开/零重发；success-resume允许共享scheduler同poll开始下一原始effect，必须保持其未确认、原bytes与至多一次RPC。
- non-test `cargo check --lib`，session8974，GREEN（1m14s）。该检查用于独立验证cfg(test)退役旧writer/driver不会掩盖生产依赖缺失，不是重复lib test。随后仅按cfg整理此次新增unused imports；不声称生产部署。
- 原A→A retry live回归复用同一已GREEN的A→B场景的显式A参数，旧测试名仍执行真实v11历史→15迁移/关闭/重开；保留原status exact bytes、request/plan/1000ms due/policy/attempt2/deadline及旧raw不变。历史valid-wire/trace/tamper的旧v11写入仅为冻结记录生成器，其第二物理连接也显式Health后再Cap；额外Health不伪造为旧record的资格receipt。
- session42955：24项22/24（编译3m24s、测试39.73s）；21项codec与原business Unknown→B零Health通过。畸形响应需要精确区分：`external_wire=Some(Missing receipt)`，不是None，也不是有效payload；保留tonic Internal、空details、无response的原语义，后续按该闭集证据修断言。新增confirmed Retry case返回ResultUnconfirmed，尚未归因生产；已减少fixture每次poll重复全库校验并加入RPC阶段诊断。此前报告将capture receipt误述成原payload，以上为更正，不改变capture实现。
- 主owner追加裁定：原v11/v12 live regression不能停留旧cold语义或仅靠新增矩阵绕过。正在逐项显式迁到15、保留原业务request/bytes/Unknown/backoff/deadline；真实旧record生成helper仍冻结在11。v12旧Health/Capabilities Unknown及provider trace/full-report目标已接显式迁移，待当前限定目标验证。

- 全调用入口闭包（源码阶段）：`GrpcMarketClient` 的普通 query、authorized unary、Capabilities、Subscribe/ListenerStatus/SetWatchlist 共用 `require_external_qualification`；`AuthorizedPreparedMacroRequest::bind_connected` 与 Capabilities bind/execute 同门。`from_channel` External 不具备 generation，不能靠调用者 supplied Channel 恢复资格。prepared cold data 不能发送业务，cold Capabilities 在连接前拒绝；显式 Health owner 才可获得活跃 client。Gateway/probe 已有显式 Health→Capabilities 次序仍保留；宏观 legacy helper同步处理 Capabilities 本地 Err，不合成远端状态。所有 production external 通道创建最终走 one-dial generation。
- Durable 入口闭包：旧 v11 writer/driver/factory只在 `cfg(test)` 保留用于生成/验证真实旧记录，生产不再存在旧 Ready→cold-send 分支；新 `macro_preparation_io_v15` / `models_preparation_io_v15` 复用原流程并要求 exact layout15，不自动迁移。v12/v14 reader/factory保留历史/本地兼容，但 external qualification新写明确要求15。Task10负责 production maintenance migration、catalog/activation与实际 owner接线，不把源码 factory存在称为已上线。
- 历史 ErrorDetail 闭包：旧状态双载体解析也使用冻结 A protobuf message decoder；继续保留 unknown protobuf字段兼容、载体冲突丢弃 provider evidence、request/operation绑定。当前 release与A结构相同，本项不伪称有独立行为RED；已有历史A/B descriptor测试/Status冲突用例扩展到B descriptor context，保留原raw bytes。当前运输的live status解析仍使用当前合同。
- 合并回归 session52268：`cargo test --lib -- grpc_client::client::external_control_attempt_tests grpc_client::client::external_mtls_attempt_tests grpc_client::client::external_native_control_tests task6_connection_journal_ task6_durable_b_ task6_connection_reopen_ task6_legacy_v11_continuation_ --nocapture --test-threads=1`，编译3m28s、测试121.94s，46/47。Task6主矩阵全部通过；唯一失败为旧 malformed generated payload用例期望`external_wire=None`，实际既有capture保留原malformed bytes。保留tonic Internal/status无response语义，只修断言为capture精确原bytes，未改生产解析。
- 运输回归前序 session31657：37项16/37，旧测试依赖cold business和非可信任意Health identity，按新合同显式Health并保留wire/status/catalog断言。session30949：control/native 17/19（编译3m27s，测试5.20s），余两处在显式Health后还断言Health为空；已修计数，后续52268此19项均通过。
- 新增A confirmed Retry→B与原business Unknown专项：保留原request/plan/deadline、1000ms backoff、attempt2和A raw status不变；Unknown case为真实v11已提交原Capabilities Begin无结果（是否poll不可知），B连资格Health也不得发送。首编译52463发现两处测试helper拼写/visibility错误（resume方法名、prepared authority字段）；已改用现有`resume_capabilities_attempt`与fixture明确authority。这不是行为RED。

- B durable矩阵：session50728（`cargo test --lib task6_durable_b_ -- --nocapture --test-threads=1`）编译3m27s、测试44.74s，2/3；qualified/rejected通过，Unknown因测试在旧lease到期前resume得到LeaseHeld。只把测试接管时间推进到旧lease到期，生产lease/deadline不改。新增完整Macro plan从A双Ready切换B、独立B Health/Capabilities后才发原data的真实mTLS用例。session4531 同命令GREEN 4/4，编译3m26s、测试57.95s；包括v11合格续跑/拒绝业务0、B资格Unknown重开0新RPC、full scope新能力后data。完整scope测试取消在data未确认，重开仍Unknown；不将其称为完整报告发布验收。

- V4 data shape 修复 GREEN：`cargo test --lib -- task6_connection_journal_ task6_connection_reopen_ task6_legacy_v11_continuation_ task6_current_control_v4_ --nocapture --test-threads=1`，session56297，编译5m41s、测试62.56s，7/7。V4 codec、首连三项、重开两项和真实v11单源续跑通过。随后B durable矩阵首编译session27834只有3个测试夹具错误（模块路径、wait helper、inspect owner），均已按既有Interface修正；这不是业务RED。

- V4 首轮集成：`cargo test --lib -- task6_current_control_v4_ macro_codec::tests task6_connection_journal_ task6_connection_reopen_ task6_legacy_v11_continuation_ --nocapture --test-threads=1`，session98394，编译4m28s、测试100.39s，25/27；全部21条codec及首连控制/两个reopen通过，first data与v11 continuation失败。定位为Live data admission仍调用只接受V3的历史shape helper，V4已确认后没有生成data link/RPC。局部允许冻结episode的V3/V4 shape，随后仍必须通过同事务layout15/current epoch/Health+Capabilities receipt；历史Ready本身不提供发送权限。

- V4 当前policy行为RED：`cargo test --lib task6_current_control_v4_ -- --nocapture`，session78488，编译6m06s、测试0.01s，0/1；严格codec报`unknown field connection_identity`，无法表达当前B的policy/epoch而不是编译失败。后继V4采用显式connection identity +当前可信policy校验；V1–V3无新增序列化字段、历史A路径不变。新Health及Capabilities写入改用当前epoch Health，而非第一个旧episode的Health；reader复核同epoch实际Health的build与每份控制结果绑定。当前session98394验证codec与journal/reopen/v11，不提前声明GREEN。

- v11 continuation/reopen/layout15 GREEN：`cargo test --lib -- task6_connection_reopen_ task6_legacy_v11_continuation_ task6_layout15_ --nocapture --test-threads=1`，session83796，编译4m36s、测试48.23s，4/4。真正旧单源恢复后完成原source final，原plan/Health bytes/业务请求不变、新Health ID、仅GlobalNews；两个重开资格与layout15精确迁移通过。后续B结果需V4并绑定当前connection receipt，当前本轮不声称B durable已闭环。

- v11 单源 continuation RED：`cargo test --lib task6_legacy_v11_continuation_ -- --nocapture`，session51001，编译7m34s、测试40.43s，0/1；真正v11旧owner生成的Health Ready记录显式迁入15后，driver返回`ResultUnconfirmed`。根因：旧reader遇到Local观察会合成full recovery，扩大原single-source请求范围。后继15只对原format2、唯一External Gateway(1)请求且无full派生事实恢复单源视图；复用同一runner collect、Live事务及qualification；source final使用后继15窄guard。当前验证session83796同时覆盖两条reopen、layout15及此continuation，不表示已经GREEN。

- 重开与首连串行验证 GREEN：`cargo test --lib -- task6_connection_reopen_ task6_connection_journal_ --nocapture --test-threads=1`，session44184，复用编译0.82s、测试65.71s，5/5。并发失败没有改动生产逻辑、deadline或历史校验；本组重型真实DB/mTLS用例后续采用串行运行。下一slice使用真正v11产生的原Health Ready记录，显式迁入15后只续原GlobalNews；测试覆盖原plan/Health/capabilities/data bytes及EconomicCalendar零调用。

- fresh Capabilities接线后 session24220：同5项，编译4m16s、测试26.90s，4/5通过；新增fresh Capabilities/Unknown通过，首连data在共享5秒watchdog超时。未改生产逻辑/时间合同。采用systematic-debugging隔离同一失败目标：`cargo test --lib task6_connection_journal_data_requires_current_capability_and_effect_link -- --nocapture`，session13680，复用编译2.79s、测试19.36s，1/1通过。并发fixture计时敏感是目前证据支持的原因，现用 `--test-threads=1` 检查同5项，不增大生产deadline。

- fresh Capabilities行为RED：`cargo test --lib task6_connection_reopen_capabilities_ready -- --nocapture`，session67461，编译3m50s、测试10.68s，0/1。两个原控制都Ready且真重开后，新Health通过但在新Capabilities前SchemaRejected。现将同一qualification begin/record Interface扩展到Capabilities，并用当前capability确认状态控制runner；session24220验证两条重开与三条journal。

- reopen Health/Unknown GREEN：`cargo test --lib -- task6_connection_reopen_health_ready task6_connection_journal_ --nocapture`，session50722，编译3m52s、测试13.63s，4/4。独立qualification先于业务、新request ID、真重开Unknown与旧Health bytes不变；首连三个journal用例无回归。下一slice为旧Capabilities已经Ready的真重开，需要新连接自己的Capabilities，不能用原Ready。

- reopen行为RED：`cargo test --lib task6_connection_reopen_health_ready -- --nocapture`，session88215，编译4m04s、测试13.61s，0/1。首次Health确认后真关闭store/client，重开driver未发送独立Health而直接SchemaRejected。测试夹具已成功确认原Health，并保留其bytes；非setup失败。随后同Live/CAS增加独立Health begin/result、qualification Unknown恢复，driver显式采用资格ticket；session50722运行该用例及此前3条journal回归。

- data Link GREEN：`cargo test --lib task6_connection_journal_ -- --nocapture`，session41560，编译3m55s、测试12.70s，3/3。真实data请求前已关联同物理generation的Health及Capabilities持久receipt；取消后Unknown，关库重开可验证。此结果不覆盖独立重资格。
- reopen测试首次编译session80461：`cargo test --lib task6_connection_reopen_health_ready -- --nocapture`，E0425，仅测试cleanup helper误名；已使用既有cleanup_external_case修正。这不是行为RED，生产实现未变。

- layout15 RED：`cargo test --lib task6_layout15_ -- --nocapture`，session2936，编译3m55s、测试1.42s，0/1；显式迁移返回 UnsupportedVersion。新后继DDL只追加连接事实表/不可变触发器/单代唯一约束并后继更新guard，不更改任何旧DDL文件、旧layout seal。GREEN命令session43241在运行，尚不声明通过。
- layout15 GREEN：同一命令 session43241，编译3m54s、测试1.55s，1/1。普通reopen不升级、显式v14→15、旧seal原样、升级后reopen精确验证、额外shadow qualification对象拒绝。下一slice单独测试RPC前资格Begin及Unknown重开，不将本项迁移通过当作驱动接线完成。
- journal 首轮编译 session8939：`cargo test --lib task6_connection_journal_ -- --nocapture` 返回E0277；测试panic试图Debug格式化RunLease。仅把诊断改为输出result.err()，未改生产逻辑；该轮不是行为RED。
- journal 行为 RED：同命令session28893，编译4m06s、测试8.94s，0/1；真实mTLS服务已收到Health，但当前epoch资格HealthBegin计数为0（要求1）。清理真实fixture后断言，没有把setup错误当RED。
- journal GREEN首次编译session9362：新增digest接线在finish_recovery作用域中误用load_on的run/layout，E0425。修复为load_on在同事务先校验资格事实，然后将已验证facts传入finish_recovery；并非行为GREEN。
- journal Begin GREEN：同命令session22815，编译4m02s、测试9.71s，1/1。首个实际Health到达前已提交当前generation/policy的资格Begin与原Health Begin；取消、关库重开后Unknown且业务0。该slice尚不包含资格Result/EffectLink或重新资格；独立控制结果也尚未改为V4。
- journal Result/Link RED：`cargo test --lib task6_connection_journal_result_ -- --nocapture`，session67677，编译4m03s、测试10.40s，0/1；Health确认后数据库Qualified Result/Capabilities Link实际(0,0)，要求(1,1)。随后增加实际completion的generation证据和同事务Result/Link；session3878运行两条journal用例，未确认GREEN前不记完成。
- journal Result/Link GREEN：`cargo test --lib task6_connection_journal_ -- --nocapture`，session3878，编译3m53s、测试10.37s，2/2。当前generation从实际completion带回；Health结果与资格Result同事务，Capabilities关联与Begin同事务，取消后原Unknown不变。首连控制链已覆盖；data及独立重资格仍待后续slice。
- data Link RED：`cargo test --lib task6_connection_journal_data_ -- --nocapture`，session56808，编译3m54s、测试9.97s，0/1；首连Health/Capabilities成功后，data仍缺当前能力关联（links=0、RPC=0）。接线同时覆盖admit、结果native投影和恢复投影，使用同一不可变当前Capabilities receipt；旧历史data仍保留原projection。session41560正在验证三条journal测试，尚未记GREEN。

- 历史decoder GREEN：`cargo test --lib -- task6_connection_qualification_ macro_codec::tests --nocapture`，session81988，编译4m06s、测试0.10s，23/23。独立A descriptor/message decoder与历史GlobalNews请求合同已接线；旧control V1/V2/V3、data source11冲突/零长、wire捕获错误、canonical/unknown/descriptor篡改、provider catalog回归及3条真实mTLS矩阵通过。新增layout15测试在该轮编译后追加，未计入这23条。

- 历史descriptor RED：`cargo test --lib task6_historical_build_ -- --nocapture`，session30102，编译3m56s、测试0.02s，0/1。隔离SQLite中的原A/V3在显式B current descriptor context下重开，仍因current descriptor比较而SchemaRejected。修复使用A归档descriptor和独立message-only解码；control保留原canonical要求，data保留原forward-compatible字段与source冲突检查，未将新descriptor自动纳入历史白名单。

- 当前policy/transport GREEN：`cargo test --lib -- task6_connection_qualification_ task6_historical_build_ --nocapture`，session25317，编译4m04s、测试0.07s，4/4。公开 A proto 归档 `contracts/external_v1_history/market.proto`，`cmp` 与当时 public proto 完全一致；build.rs 独立生成 message-only 历史模块，无 RPC client/server。此轮未声明历史decoder接线完成。

- B当前policy RED：`cargo test --lib task6_connection_qualification_new_b -- --nocapture`，session84543，编译4m00s、测试0.08s，0/1。A→同URI独立B时A policy拒绝B/业务0成立，但连接仍硬调public pin，显式B policy的新Health被拒绝。最小修复为session固定自己的编译可信policy实例；测试B仅使用cfg(test)显式实例，无环境变量/首次响应学习信任。

- One-dial GREEN：`cargo test --lib task6_connection_qualification_ -- --nocapture`，session 32689，编译3m55s、测试0.05s，2/2。cold未资格拒绝；同URI A→独立B断流后旧Channel/clone二次dial=0，B Capabilities/data=0。该轮不包含其后追加的 B policy 测试。

- Reconnect RED / Cold GREEN：session 82194，编译4m59s、测试0.05s，1通过/1失败。Cold Capabilities=0通过；普通 tonic transport 在 A Health 后强制断流到独立 B，同一 clone 的后续 Capabilities 仍至少一次成功，`assert!(failed)` 真实失败。现在仅将 prepared connector 改为 one-dial 再验证同目标。

- Cold GREEN 首次编译 session 96226：`cargo test --lib task6_connection_qualification_ -- --nocapture` 因 `src/search_service/macro_news/legacy.rs` 的旧 Capabilities 调用未展开新 Result 而退出101（E0308，1个错误）；这不是行为 RED。该调用已同步为本地错误分支，不合成远端回执。
- Reconnect RED 夹具：新增字节级 TCP switch，在不改变 URI/TLS authority 的情况下关闭全部旧 relay，转向第二个独立 tonic+mTLS 服务。session 82194 保持普通 `Endpoint::connect`，先观察自动重连漏洞，再接 one-dial connector；不是伪造客户端 revoked 状态或只修改同服务响应字段。

- Cold Capabilities RED：`cargo test --lib task6_connection_qualification_ -- --nocapture`，session 30238，编译 3m45s、测试 0.03s，0/1。真实本地 tonic+mTLS 服务在未发 Health 情况下接收 Capabilities=1（要求0），fixture 正常关闭后断言失败。随后引入共享 connection generation、one-dial connector、所有 external 业务调用 gate；Capabilities 本地未资格返回 `Err`，不伪装 ConnectUnavailable 或远端 Status。

- 历史 policy GREEN：`cargo test --lib -- task6_historical_build_ grpc_client::build_identity::tests new_external_control_results_bind_request_and_verified_health_build --nocapture`，session 58934，编译 3m36s、测试 0.06s，4/4。覆盖历史 A 在 B trust context 可读且不被 B 当前资格接受、current pin 缺失/篡改拒绝及原 control capture 绑定回归。descriptor、连接代次和 layout15 尚未在此 slice 覆盖。

按垂直切片推进：历史 policy/descriptor → cold/reopen 零业务 → same-URI 实际断连 → 独立持久 qualification 与旧布局迁移/Unknown → v11/v14 来源迁入15与全部调用闭包。每次测试结束记录真实 RED/GREEN；编译错误不冒充业务 RED。不并发 Cargo、不跑无关全量。

- 历史 policy RED：`cargo test --lib task6_historical_build_ -- --nocapture`，session 85092，编译 3m38s、测试 0.02s，0/1。冻结 A V3 bytes 写隔离 SQLite、关闭重开后，B trust context 的历史 project 返回 `SchemaRejected`。测试没有修改环境/global pin，也未用 B 当前 helper 再生成 A 历史；这一 slice 仅证明 codec/reopen，不冒称完整 durable A→B 迁移矩阵。
- 最小修复：提取实例化 `BuildIdentityTrust`，V1–V3 历史使用 `contracts/external_v1_history/bundle-20260917.1.json` 的显式冻结发布 policy；当前资格仍使用当前 bundle pin。旧 V3 未保存 policy digest，因此不猜其它旧版本、不把任意响应加入可信目录；layout15 新结果后续必须单独绑定 current policy receipt。当前 archive 来源 metadata 原文件 SHA256 为 `81925a054c9b61ac987f02efc1d6c60805481bc400d99ae7320506a8e91c5cae`（归档新增终止换行，JSON 值相同）。
