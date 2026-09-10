# P-02 真实业务影子接线：当前证据与实施前置

日期：2026-09-10。P-02业务源码引用更新至提交 `298ab0d7b089a8c9173a90740b40f6d2148147bf`，原观察、冻结提案和dispatcher未在后续W17批次改变。W17泛型业务提案机制已完成至 `9b2a5df7033cc490ffe9ed5749073eada3f09653`，26项定向测试及限定复审通过，过程例外见[实施记录](implementation-shadow-business-proposals-2026-09-10.md)。本记录承接已有能力，避免下一任务重复实现；不是新的影子adapter已交付、不是运行期或生产验收。W18请求入口与P-02选集拒绝诊断局部任务均完成；诊断的10项定向测试和独立审查见[实施记录](implementation-auction-selection-diagnostics-2026-09-10.md)，不能据此关闭下面的完整接线缺口。

## 已有能力与真正缺口

| 位置 | 当前已实现 | 下一步必须补齐 |
| --- | --- | --- |
| [目录 Unit](push-capability-catalog.v1.json#L8813) | P-02 对应 `MU-auction-volume`，producer 为 `auction-volume` | 不把测试示例 `MU-p02` 等任意文本当正式注册；该目录是冻结历史，旧的“双次加载”说明不代表当前实现 |
| [观察对象](../../src/market_analyzer/limit_up.rs#L67)、[真实采集入口](../../src/market_analyzer/limit_up.rs#L497) | 私有字段保留涨停池、名称分片、原请求 hash、实际审计回执及原始股票 | 将同一次观察绑定到一次准备事实，不能重新采集以补证据；本地回执不是生产来源身份认证 |
| [真实 loader](../../src/bin/monitor/push_templates.rs#L6165)、[实际 main 消费](../../src/bin/monitor/main.rs#L9710) | loader 从观察的股票生成快照，main 保留完整 tick、typed选择错误，P-02 与后续持仓检测借用它 | 新路径必须接在这个实际调用点，不能只增加无人调用的模拟入口 |
| [横幅捕获](../../src/bin/monitor/push_templates.rs#L256) | `CapturedBanner` 已冻结展示文本；捕获会读外部账户/估值说明 | 把捕获放在 old/new 纯投影之前，两侧只用同一结果；不能在影子 callback 内再次 `capture/render` 触发外部读取 |
| [完整业务提案](../../src/bin/monitor/push_templates.rs#L6187)、[判等](../../src/bin/monitor/push_templates.rs#L6218) | 同时包含 message、有序逐票 records、notified_codes；价格按 `to_bits` 比较，records 字段及顺序均参与 | 影子比较必须覆盖这三部分，而不只比较可见文案或文案 hash |
| [当前 dispatcher](../../src/bin/monitor/push_templates.rs#L6295) | 先冻结一次提案；sink 成功且所有 recorder 成功后才推进通知集合 | 引入比较后，发送/记录/集合推进仍消费被比较的同一旧提案；禁止比较完重新 prepare 一份 |
| [W17同次业务入口](../../src/monitor/push_job/shadow.rs#L439)、[绑定校验](../../src/monitor/push_job/shadow.rs#L611) | 共用内核、同context/Arc facts和八类拒绝能力；比较本次实际传入的P，保留旧原对象。基础与提案存在性双重错误均留证据，原入口兼容 | 还没有P-02实际old/new adapter；必须在真实调用处传完整PreparedAuctionVolumeDispatch，而非空载荷或两份相同结果的克隆 |
| [PreparedPush](../../src/monitor/push_job/projection.rs#L713) | 绑定 intent/decision/context/facts/semantic/rendered bytes | 该对象没有 P-02 的逐票写库记录和通知集合，不能仅因其相等而断言完整 P-02 行为相等 |
| [上下文工厂](../../src/monitor/push_job/context.rs#L189)、[投影构造](../../src/monitor/push_job/projection.rs#L277) | 非 test 的 factory/type/构造逻辑已经存在，但 binding/input 字段和有效构造路径受限 | 缺的是可信注册与实际运行输入进入这些构造路径的 interface，不是“所有类型只在 cfg(test) 存在” |
| [机器注册字段](../../src/monitor/push_job/catalog.rs#L191) | Unit、completion owner、producer、occurrence family、phase 已有权威目录关系 | source contract/version、模板、audience、completion policy 不能从测试常量推定为生产注册；需明确实际注册来源和绑定 |

## 不能只比消息：已有独立反例

[完整提案测试](../../src/bin/monitor/push_templates.rs#L18491)逐字断言文案、两条 records 的全部业务字段和通知集合；[价格/指标/集合差异反例](../../src/bin/monitor/push_templates.rs#L18548)明确证明，价格改变或隐藏在显示舍入后的指标变化，可以让消息完全相同而业务记录不同。下一任务须复用这些独立期望并从实际影子 interface 检测差异，不能改成“同一个 prepare 调用两次，所以结果相等”。

业务提案的精确比较应与同次 W17 执行绑定，并保留旧提案供实际发送。可复用现有 `PartialEq`，但单独在测试比较两个提案、或由 caller 自报一个 payload hash，都不能证明运行时比较了完整实际输出。接口还须确保回调失败、未执行、任一拒绝端口非零时不能生成 Match；Debug 仅输出类型、数量和差异类别，不输出提案正文。

历史1931014核对时，ShadowObservation没有业务记录/通知集合载荷；execute_shadow只返回ShadowReport，不返回可供dispatcher消费的旧业务提案。这确定了需补的机制：比较器接收两侧真实提案、在同次执行中比较，并保留原旧提案的所有权；不能在外部拼接自报“相等”bool或事后重新prepare。原report的is_match仍只证明原接口覆盖的语义与所提供拒绝能力，未证明全部P-02业务一致或允许发送。此段为历史缺口，不以旧行号链接冒充当前源码位置。

这一机制已按[同次完整业务提案比较计划](../superpowers/plans/2026-09-10-shadow-business-proposals.md)完成至9b2a5df：保留原execute_shadow及公开闭集合同，新入口比较实际传入载荷并移动保留旧输出。基础无效同时缺提案的漏证据已修复，最终26项测试及限定复审通过；开发验证过程例外见[实施记录](implementation-shadow-business-proposals-2026-09-10.md)。[take_legacy_proposal](../../src/monitor/push_job/shadow.rs#L391)只交回普通业务数据，不以整个report的Match授予/撤销owner，也不是发送capability。真实P-02完整类型、两个adapter、同次捕获与dispatcher消费仍须接线审查，来源认证没有由此完成。

## 实施依赖顺序

2026-09-10 认证前置已按8ad4f9f重新独立核对，见[运行时认证接线缺口](runtime-auth-integration-gap-2026-09-10.md)。生产身份、生产broker和有效context注册均缺真实成功入口；现有类型/方法本身并非全在测试条件编译。外部平台/受保护根待确认不等于全部代码只能等待，但不能通过开放私有字段或测试常量完成本节所需的可信注册。

### 注册入口不能靠放宽构造器完成

以下接点按20f215d再次只读核对，属于真实adapter计划的必需输入，不是又一份已实现注册能力：

| 接点 | 已有检查 / 尚缺输入 |
| --- | --- |
| [目录查询](../../src/monitor/push_job/catalog.rs#L191) | 可获得producer所属Unit、owner、occurrence family及phase；结构中没有来源合同/version、audience、模板或completion policy，不能从目录查询凭空补出这些批准值 |
| [RunContextFactory](../../src/monitor/push_job/context.rs#L189) | binding/input的字段仍私有；build_context校验family、Test命名空间run_id及trigger匹配。构造器可见性不等于真实性认证，不能改成公开任意字段入口来接main |
| [一次捕获入口](../../src/monitor/push_job/context.rs#L267)、[capture_once](../../src/monitor/push_job/facts.rs#L649) | context与预期来源合同先绑定，再至多一次执行acquire并封存事实；实际接线须让原观察进入这次捕获，不能在已有采集之外调用第二个provider补事实 |
| [DecisionProjector](../../src/monitor/push_job/projection.rs#L277) | 现有构造校验Unit并绑定context摘要；ProjectionBinding还需audience、kind/sub_kind、owner、policy与模板。输入值必须来自同一注册及运行证据，不是测试fixture |
| [SourceRef](../../src/monitor/push_job/facts.rs#L49) | 公共构造器只组合类型化引用，不核验外部provider身份；能构造引用不代表已认证来源，真实审计receipt也不能代替该认证 |
| [部署候选](../../src/push_foundation/activation_deployment.rs#L1)、[当前记录读取](../../src/push_foundation/readiness_query.rs#L1) | 都明确只返回未认证候选；不能因已通过目录闭合、磁盘重读或摘要一致，就把它们直接提升为context需要的批准generation/build/业务日来源 |

这符合[RFC RunContext逐字段合同](push-system-implementation-rfc.md#类型runcontextproposed)：generation来自已批准owner栅栏、build与activation校验身份一致、业务日由交易日authority捕获。后续可开发受约束的校验/传递接线，但没有真实根、来源与批准值时必须保持不可发放生产权限；不把“所有类型都不存在”或“只差公开一个new”当剩余工作描述。

### 真实adapter与上线验收顺序

1. **完成真实注册/捕获 interface 的具体设计。** 核对正式 Unit/producer/occurrence/owner 关系，将 source、模板、audience、completion policy 与部署/来源证据逐项绑定。不得放宽私有字段或增加任意“成功构造器”来让跨 crate 测试通过。这里不要求先完成全部 W15，但来源/部署声明与真正认证必须分清；缺实际配置的生产权限不可由本地默认值补齐。
2. **一次捕获，两个真实纯 adapter。** 同一个 RunContext、共享 PreparedFacts、同一来源观察、通知集合起始快照与 CapturedBanner 进入旧/新路径。两侧使用实际业务逻辑及 W05 投影，比较完整决策和业务提案；不能将 legacy 的一个结果克隆两份冒充两条执行，也不能二次读取行情、账户或墙上时钟。
3. **接实际消费与拒绝能力。** 比较结果绑定本次输入/Unit/输出，现有 dispatcher 消费同一旧提案。八类效果在影子路径实际接拒绝能力，计数先于拒绝；纯回调不能沙箱化任意全局 I/O，未纳管的旧全局调用不得获得“零调用”证明。
4. **接 W16 当前激活证据和共同 fence。** 影子 actor 无效果权限，Match 也不授予发送或晋级。初始 Disabled/Shadow 下旧 owner 是否保留、排空后的关闭及 rollback 新代恢复，严格沿[W16 已澄清合同](activation-contract-decisions-2026-09-08.md)，不能把整个 Unit 一律关掉，也不能凭影子失败或成功自行改 owner。
5. **形成逐 Unit 验收，再进入灰度。** 合成 Ready 样本只证明代码行为；真实源、真实批准、当前 fence、端口纳管和实际观察窗口分别要有证据。以上各层未齐，不能关闭 W17 或把本 Unit 计为已迁移。

## 必须保留的缺源与拒绝语义

当前真实投影仍明确设置 [volume_ratio: None](../../src/market_analyzer/limit_up.rs#L295)，[P-02 selector](../../src/bin/monitor/push_templates.rs#L6041)仍要求有限正量比、有限涨跌幅和有限正价格。保留来源观察不等于获得量比。

- provider 证实空池，只能依照观察对象自身的 `VerifiedEmpty` 事实生成 NoData 证据。
- 非空池却缺量比、字段非法或来源未认证，不能当 VerifiedEmpty；现在的 `snapshot: Err(AuctionVolumeSelectionError)` 已保留空源和非空不可选的区别，仍不能不加来源验证地映射为 NoData。
- 全部已通知造成的空选集和缺输入造成的空选集，已有[只读计数与严格事实判断](../../src/bin/monitor/push_templates.rs#L5961)可区分；后续adapter仍须依既有政策/输入合同决策，不能将选择事实当领域完成或真实空池。
- 任何新的跨源补量比仍需[量比来源合同](auction-source-evidence-gaps-2026-09-08.md)中列出的产品决定与提供方事实；不得新增隐式 MarketStatistics join，也不得将原 receipt/hash 重新命名为认证。

## 下一任务的可观察验收

验收至少包括：同实例/同次采集；一份横幅；完整提案中消息不变但价格、原始指标、记录顺序、通知集合变化均被拒绝；全量 Ready/NoData/阻断分支的真实 W05 绑定；第二次 provider/账户读取被拒绝；八类实际接入能力的尝试计数；失败不推进通知集合；最终消费与被比较提案一致；受控测试可以 Ready 但真实缺量比不得 Ready。

最终测试命令与文件 ownership 须在具体 adapter/注册 interface 确定后写入正式实施计划，并逐项核对测试是否会调用全局数据库、网络或 dispatcher 日志。当前未运行这些新测试，也未创建这套实现。已有[冻结准备](implementation-auction-frozen-preparation-2026-09-08.md)、[来源保留](implementation-auction-source-observation-2026-09-08.md)、[W17 内核](implementation-w17-results-2026-09-08.md)的完成状态不重开；只补它们之间真实缺失的接线。

其中无需先签发生产权限的输入事实保留已完成[选集拒绝诊断计划](../superpowers/plans/2026-09-10-auction-selection-diagnostics.md)：真实选择器/tick的String失败已替换为可区分空源、缺字段、有效行已通知的结构化事实，原成功发送结果不变。源码298ab0d经10项纯测试和独立Spec/Quality审查通过；它不是完整adapter/注册计划，也不解决真实缺量比或认证。
