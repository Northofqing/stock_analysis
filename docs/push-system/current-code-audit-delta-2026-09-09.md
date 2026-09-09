# 当前代码审计增量与历史证据边界

日期：2026-09-09。状态：PROVISIONAL；当前源码核对基线为 `aef7972965f610ed418049593dfff1d55341772e`，历史目录基线为 `07781bf386aafdf202851ae928efee8920387058`。本记录是当前代码审计的增量分析，不是新机器目录已发布、Unit已晋级或生产已验证的证明。

## 为什么不能原地刷新旧目录

旧目录不只是一份展示文件：RFC元数据及校验器固定其SHA，WBS固定整体SHA与各Unit快照，运行时注册表也通过 `include_bytes!` 和精确SHA绑定它。只更新旧JSON会破坏这些合同。因此保留历史原字节，后续追加独立的当前审计版本；不把审计版本自动升级为运行时或RFC权威。

| 消费者 | 当前绑定证据 |
| --- | --- |
| 规范RFC | [RFC元数据](push-system-implementation-rfc.md#元数据)、[rfc_spec.rb](../../scripts/architecture-docs/rfc_spec.rb#L14)：历史baseline与两份JSON的固定SHA |
| WBS | [wbs.rb](../../scripts/architecture-docs/wbs.rb#L91)：整体catalog SHA；第160–166行逐Unit快照 |
| 运行时MachineCatalog | [catalog.rs](../../src/monitor/push_job/catalog.rs#L14)：固定文件、SHA、65 kinds / 102 producers / 52 Units；[既有精确测试](../../src/monitor/push_job/tests.rs#L2263) |
| 现有Catalog校验 | 工具源码7a150b2：[catalog.rb](../../scripts/architecture-docs/catalog.rb#L50)核对当前字节/符号；[git_errors](../../scripts/architecture-docs/catalog.rb#L356)同时核对历史Git tree。批量读取及错误聚合修复不改变旧快照与当前树不一致时失败的规则 |
| 八份冻结来源 | [rfc_inputs.rb](../../scripts/architecture-docs/rfc_inputs.rb#L9)与[rfc-input-manifest.v1.json](rfc-input-manifest.v1.json)：这是另一条历史来源链，不因新增当前审计而解冻 |

## 107条错误的真实构成

原始 `check-catalog.rb --draft` 输出共有107条内容错误，不是107个独立业务缺陷。实测当前文件集为548，历史manifest为469，新增79、删除0。主线已直接读取当前树与JSON复核文件数量；语义定位由两路只读核查完成。

| 分类 | 条数 | 处理方式 |
| --- | ---: | --- |
| 已定位的八项语义变化 | 15 | 一个旧symbol缺失；其余七项各有正文SHA和行号变化。必须更新语义与依赖关系，不能只替换hash |
| 正文SHA相同、仅行号变化 | 73 | 由正式locator重算行号，不重写未变正文的业务含义 |
| 完整文件SHA变化 | 18 | 归因到现有实现提交，再补新增声明及依赖边；与上两类并非不同文件集合 |
| 文件集合变化 | 1 | 表示79个新增文件，不是一个新增文件。生产文件46个、测试文件33个 |
| 合计 | 107 | 八项之外剩92条原始错误，不是99个独立语义问题 |

73项纯行号变化分布：main.rs 2项；push_templates.rs 65项；data_gateway/review.rs 1项；durable_delivery/coordinator.rs 4项；event/mod.rs 1项。其正文SHA仍与历史证据完全一致。原80条行号错误中的另7条属于上表已变化正文。

新增46个生产文件分为 `src/monitor/push_job*` 10个与 `src/push_foundation/*` 36个；33个测试文件只进入完整文件manifest，不能伪装成生产业务证据。注册关系见[src/lib.rs](../../src/lib.rs#L58)、[src/monitor/mod.rs](../../src/monitor/mod.rs#L32)、[push_job.rs](../../src/monitor/push_job.rs#L3)与[push_foundation/mod.rs](../../src/push_foundation/mod.rs#L3)。模块存在不等于全部producer已切换。

## 八项正文变化的业务含义

| 历史evidence ID | 当前定义/消费者 | 必须保留的边界 |
| --- | --- | --- |
| auction-source | [load_auction_volume_tick_real](../../src/bin/monitor/push_templates.rs#L6059)、[main调用](../../src/bin/monitor/main.rs#L9710)、[来源审计](../../src/market_analyzer/limit_up.rs#L497) | 不是旧loader一对一更名；完整原始列表、snapshot Result及来源观察分开，筛选先于Top10 |
| auction-volume | [dispatcher](../../src/bin/monitor/push_templates.rs#L6232)、[准备](../../src/bin/monitor/push_templates.rs#L6132)、[执行](../../src/bin/monitor/push_templates.rs#L6189) | 消费一次采集的snapshot和冻结提案；所有逐票记录成功后才推进集合。仍有旧bool投递语义，部分入池失败不回滚，不构成exactly-once |
| counted-envelope | [DeliveryEnvelope](../../src/durable_delivery/model.rs#L802)、[Foundation调用](../../src/push_foundation/generic_transport.rs#L365) | legacy空绑定/省略序列化与Foundation decision绑定是两个身份域，不能沿用同一旧hash说明 |
| monitor-loop | [主循环](../../src/bin/monitor/main.rs#L8796)、[P-02入口](../../src/bin/monitor/main.rs#L9708)、[持仓消费](../../src/bin/monitor/main.rs#L9954) | snapshot错误阻止P-02，但完整原始列表仍可供持仓消费；保留观察不是生产来源认证完成 |
| startup-all-pending | [入口](../../src/durable_delivery/coordinator.rs#L3991)、[共享实现](../../src/durable_delivery/coordinator.rs#L4035) | Global恢复与严格decision作用域共用实现，不能只锚定薄wrapper |
| startup-begin-attempt | [入口](../../src/durable_delivery/coordinator.rs#L5094)、[模板选择](../../src/durable_delivery/coordinator.rs#L5236) | Foundation template绑定与旧PushKind映射共存，不替代attempt/lease/fence准入 |
| startup-expired-attempt | [恢复](../../src/durable_delivery/coordinator.rs#L5537)、[作用域筛选](../../src/durable_delivery/coordinator.rs#L5552) | decision限制发生在候选选择前，Global与局部恢复范围不同 |
| startup-list-deliverable | [入口](../../src/durable_delivery/coordinator.rs#L6183)、[查询](../../src/durable_delivery/coordinator.rs#L6200)、[hydration](../../src/durable_delivery/coordinator.rs#L6250) | summary和hydration都受同一scope约束，仍须区分可投递、活跃租约与人工未决 |

## 七个既有文件中的新增语义锚点

下列九个声明已由正式 `RustEvidence.locate` 在当前源码重新定位，全部kind为 `rust_fn`；完整SHA和行号的原始JSON保存在隔离树 `.planning/2026-09-06-push-foundation-runtime/current-audit-anchor-candidates-2026-09-09.json`。它们是拟纳入current审计的候选，不会自动进入旧规范目录。

| 文件及声明 | 当前行号 | 消费者与应如何表述 |
| --- | --- | --- |
| [calendar.rs](../../src/calendar.rs#L439)：`verified_a_share_calendar_authority_hash` | 439–447 | activation_deployment.rs:168消费；仅证明calendar authority进入Foundation模块，不证明生产批准/owner/受保护根已经成立 |
| [market_capabilities.rs](../../src/data_gateway/market_capabilities.rs#L304)：`security_identities_observation` | 304–321 | limit_up.rs:452–492的auction-source输入边；保留真实bridge/provider路由 |
| [同文件](../../src/data_gateway/market_capabilities.rs#L324)：`retain_security_identities_observation` | 324–346 | 配对证明request hash、provider及BR-159 receipt，不能只列前一个薄调用 |
| [review.rs](../../src/data_gateway/review.rs#L925)：`audit_limit_up_projection_in` | 925–987 | limit_up.rs:497–516消费；覆盖pool/name-shard join、canonical evidence及BR-159追加 |
| [event/dispatcher.rs](../../src/event/dispatcher.rs#L790)：`read_authoritative_year` | 790–857 | event/mod.rs:996消费；existing-only年度权威链读取，不创建缺失链来伪造空记录 |
| [event/mod.rs](../../src/event/mod.rs#L975)：`requery_news_flash_window_terminal_with` | 975–1186 | dedicated_transport.rs:146–150消费；精确业务日/窗口terminal reader，不等于N02 producer已切换Foundation |
| [durable_delivery/schema.rs](../../src/durable_delivery/schema.rs#L942)：`migrate_schema_v4_to_v5` | 942–1127 | schema.rs:105/114/122/129调用；独立基础设施证据，保留rowid与predecessor自外键，不硬挂到某个producer |
| [data_acquisition_audit.rs](../../src/database/data_acquisition_audit.rs#L99)：`read_verified_acquisition_audit` | 99–110 | 本次源码引用核对除定义外只找到测试调用，未发现非测试消费者；不能算auction-source已接线 |
| [同文件](../../src/database/data_acquisition_audit.rs#L120)：`read_acquisition_in_transaction` | 120–129 | 同上；caller-owned transaction接口存在，不代表实际恢复链已经消费 |

七个适合生产调用边或基础设施分组的声明，与最后两个未被生产消费的reader必须分开标注。专用N02 adapter仍保留库级/未完成生产接线边界；受保护身份、批准、owner切换和受控真实传输不由本次代码审计替代。

## 新增Foundation模块的八组架构证据

新增46个生产文件已分层归类，选出29个代表性声明；主线通过正式locator逐个提取，10807终态exit0、无定位失败，完整记录见同规划目录 `current-architecture-anchor-candidates-2026-09-09.json`。下表证明代表性模块合同及库内依赖，不是46个文件的逐行完整审查，也不是79个文件已经纳入正式current manifest。

| 分组 | 代表性声明与代码证据 | 能证明什么、不能证明什么 |
| --- | --- | --- |
| 目录/身份/上下文 | [MachineCatalog impl](../../src/monitor/push_job/catalog.rs#L236)、[derive_intent_id](../../src/monitor/push_job/identity.rs#L418)、[RunContext impl](../../src/monitor/push_job/context.rs#L319) | 纯库合同存在；Foundation消费类型，不等于monitor已调用新调度链 |
| facts/project/policy/shadow | [PreparationCapture impl](../../src/monitor/push_job/facts.rs#L633)、[DecisionProjector impl](../../src/monitor/push_job/projection.rs#L275)、[evaluate_completion](../../src/monitor/push_job/policy.rs#L829)、[execute_shadow](../../src/monitor/push_job/shadow.rs#L279)、[classify_durable_state](../../src/monitor/push_job/delivery.rs#L593) | 捕获/投影/结果分类/影子合同存在；不是各Unit已提供真实业务adapter或生产shadow证明 |
| 持久化/迁移 | [BusinessIntentStore impl](../../src/push_foundation/intent_store.rs#L1280)、[apply_to](../../src/push_foundation/migration.rs#L101)、[inspect_raw_activation_facts](../../src/push_foundation/activation_store.rs#L45) | 公开库级入口；[模块声明](../../src/push_foundation/mod.rs#L1)明确没有选择或迁移生产DB |
| activation候选/部署集合 | [apply_activation_candidate](../../src/push_foundation/activation_transaction.rs#L108)、[construct_activation_deployment_set](../../src/push_foundation/activation_readiness.rs#L373)、[project_owner_admission](../../src/push_foundation/activation_owner.rs#L77) | 私有库级候选路径；不是生产身份、批准映射或真实owner已生效 |
| effect围栏/IPC | [EffectBroker impl](../../src/push_foundation/activation_fence.rs#L636)、[IPC impl](../../src/push_foundation/activation_fence_ipc.rs#L95) | production构造当前直接返回ProductionRefused；generic/business effect绑定入口仍为cfg(test)，不能用于证明生产授权完成 |
| readiness/调度/快照 | [ReadinessAssessment impl](../../src/push_foundation/operational_readiness.rs#L335)、[PhaseScheduler impl](../../src/push_foundation/phase_scheduler.rs#L479)、[CandidateReadinessSnapshot impl](../../src/push_foundation/readiness_snapshot.rs#L205)、[append](../../src/push_foundation/readiness_store.rs#L172)、[initialize_database](../../src/push_foundation/readiness_store_schema.rs#L147) | 评估与存储能力存在；当前生产进程的就绪查询、真实来源和所有Unit消费者尚不能据此宣称接线 |
| authority/transport | [verify_terminal](../../src/push_foundation/terminal_authority.rs#L176)、[execute_current](../../src/push_foundation/generic_transport.rs#L237)、[verify_p01_dedicated](../../src/push_foundation/dedicated_transport.rs#L154)、[verify_n02_dedicated](../../src/push_foundation/dedicated_transport.rs#L203) | adapter依赖既有runtime类型；反向生产调用和真实受控传输不能由callee存在证明 |
| finalizer/recovery/指标 | [commit_accepted_finalization](../../src/push_foundation/business_finalizer.rs#L814)、[reconcile_current](../../src/push_foundation/reconciler.rs#L337)、[inspect_finalization_metrics](../../src/push_foundation/finalization_metrics.rs#L393)、[inspect_finalization_sla](../../src/push_foundation/finalization_sla.rs#L271) | 最终化/读取接口存在；reconcile_current经测试绑定effect进入，指标/SLA没有发现生产caller，不等于运行闭环已上线 |

关键否定证据经主线直接读取复核：[production固定拒绝](../../src/push_foundation/activation_fence.rs#L637)、[generic effect测试绑定](../../src/push_foundation/activation_generic_effect.rs#L394)、[business effect测试绑定](../../src/push_foundation/activation_business_effect.rs#L448)、[reconcile_startup仅测试编译](../../src/push_foundation/reconciler.rs#L276)。不能通过删掉这些拒绝或cfg边界来冒充补齐真实认证。

剩余supporting文件仍必须进入完整文件manifest：公开数据/存储6个文件、activation部署/围栏12个、readiness/scheduler 11个、authority/transport/finalization 7个，加push_job 10个共46。代表性symbol未覆盖到的codec、SQLite I/O、导出与模块声明只能按文件级证据表述，不提升为已验证的业务效果。

现有locator只支持 `rust_fn/rust_enum/rust_impl/rust_mod`；trait/struct/pub use不在该合同内，外置 `mod name;` 也没有可定位主体。新架构证据须有独立的非迁移引用域；把它们硬塞进旧producer关系既会误导业务含义，也绕不开旧Catalog对未引用evidence的拒绝。callee SHA不证明caller、cfg可达性、对象实例化或生产数据库选择。

## 尚未闭合的工作

- 46个新增生产文件已有分组及代表性声明定位；正式current格式中的架构引用域、supporting文件闭包和业务关系仍待实现/验证，不能只把79个新文件SHA写入manifest就宣称全部语义已审计。
- current机器目录及其与历史版本的绑定、完整文件/符号/枚举/引用验证尚未实现；107项既有错误尚未消除。
- 蓝图current内容/第二HTML、兼容wrapper、两目标统一检查及实际CI执行仍待交付；原RFC、WBS及runtime catalog不自动升级。
- 本次未运行生产monitor、生产数据库或真实推送，也没有触发CI、发布或批准任何Unit。
