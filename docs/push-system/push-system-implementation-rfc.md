# 推送系统实施 RFC

状态：**PROVISIONAL**。版本：`push-system-rfc-v1`。本文定义 PROPOSED 应用合同，
不代表运行时代码已经实现、部署或取得远端回执。Task2 仅交付领域类型、应用结果、
源码映射和 ReasonCode；DDL/恢复、运行门禁、WBS 分别属于 Task3/4/5，本文不宣称
这些后续任务已经完成。[Q:56] [Q:61] [Q:69] [Q:108]

## 元数据

```json
{
  "schema_version": 1,
  "status": "PROVISIONAL",
  "version": "push-system-rfc-v1",
  "source_baseline": "07781bf386aafdf202851ae928efee8920387058",
  "input_manifest_sha256": "6a74428f1cc18cc1b0800ab86be19e3d8afdaafd0107d7656a2d3a5857f18aab",
  "catalog_sha256": "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3",
  "evidence_manifest_sha256": "54dc705961da7a6deb458009d2125ee612257d82bad3c14b65d25642e09b64fa",
  "decisions_sha256": "55354916a4b03401afa771e2f4e149bc1189222fc3c76d89aeb5ad79c086e794",
  "counts": {"kinds": 65, "producers": 102, "units": 52, "evidence": 195, "mapped": 26, "durable_kinds": 23, "unmapped": 39, "states": 14},
  "status_counts": {"ACTIVE": 36, "INACTIVE": 22, "STARVED": 5, "OPT-IN": 2}
}
```

## 范围与事实权限

规范优先级：已批准的 Q1–Q108 > 冻结的 65-kind/102-producer/52-Unit 目录及
195 条 symbol 证据 > 冻结 Rust 映射/状态 > 蓝图提案与最近样本。CURRENT 仅表示
元数据指定基线的源码接线，不表示生产已激活；PROPOSED 表示未来合同。
历史 37/24/2/2、35 Unit 和旧估算不属于本基线。PaperBuy/Watchdog 仍是被排除的
原工作区新增项；枚举外 CLI 生产者仍属于当前目录。[Q:21] [Q:30] [Q:59] [Q:65]
[Q:93] [evidence:push-kind] [unit:MU-cli-single]

实施顺序为 Foundation → 原子 Unit 切片 → 尾部清理。四时段仅是 Epic，
不是物理完成所有者的切换边界。本文不证明 Unit 晋级、HTML/CI 发布、运行时部署
或用户已收到消息。[Q:16] [Q:17] [Q:23] [Q:26] [Q:42] [Q:74]

## 阅读导航与规范化规则（PROPOSED）

后续章节依次定义字段、JobDecision、DeliveryResult、完成分支、身份与终态合同、
CURRENT 映射和状态、ReasonCode、适配器一致性。引用使用方括号内的
`Q:编号`、`unit:ID`、`producer:ID`、`evidence:ID`；校验器实际解析并核对
冻结文档中的编号和身份，不以关键词出现代替引用检查。[Q:59] [Q:66]

`prepare(RunContext) -> PreparedFacts -> project() -> Ready(PreparedPush) ->
业务 intent -> DeliveryCoordinator -> VerifiedTerminalRef -> Finalizer(CompletionPolicy)`。
不发送的 JobDecision 走独立完成提案，不构造终态引用。调用方不得解释 bool/Ok
来推进完成，也不得在拒绝后绕过 coordinator 直接发送。[Q:27] [Q:34] [Q:78] [Q:86]

规范化编码 v1 先写入类型名与 schema/version 组成的域标签，再编码 UTF-8 JSON：
对象键按字典序排列，禁止重复键、无关空格和末尾换行；整数用最短十进制表示，
禁止浮点数与非有限数；字符串按 JSON 规则转义，捕获后不再做文本归一化。
日期为经过校验的 YYYY-MM-DD；UtcMicros 为 i64 UTC 微秒；Sha256 为 64 位小写
十六进制，GitSha40 为 40 位小写十六进制。ExactBytes 对原始字节直接求哈希，
在外层规范化对象中以 SHA 和字节长度表示；数组保留捕获顺序，Option 显式编码
为 null，不得省略。[Q:10] [Q:19] [Q:33] [Q:40] [Q:72] [Q:89]

NonEmptyText 与身份值必须满足注册来源合同的长度和内容约束。OccurrenceId 是
(business_date, 注册的 occurrence 家族, family_key) 元组；外层身份另绑定命名空间
和 Unit，跨业务日复用显示名称不能碰撞。SourceRef 保存 provider、外部项目身份、
来源合同身份及不可变内容哈希，不代表发送权限。这些类型必须经构造校验，
不是不受约束的 String 别名。[Q:16] [Q:59] [Q:89]

字段表中每项均必填，包括显式 Option。规范化列使用三个固定值：“纳入”表示纳入
该类型的规范化对象；“外部原始字节”表示原始内容保持不变，外层编码其长度和 SHA；
“派生且排除自身”表示不纳入自己的哈希材料，下游对象通过其摘要绑定。
校验器逐字段固定此列，不能互换合法模式。每行继承本表创建者、消费者及共同禁止
行为：构造后不得修改，不得绕过校验反序列化，不得伪造权威结果。跨字段绑定不一致
必须拒绝，不能静默修补。重试须读取首次冻结的 RunContext/PreparedFacts/PreparedPush，
不得为同一 intent 重新捕获时钟、来源响应、模型输出或模板。[Q:10] [Q:33] [Q:72]
[Q:78] [Q:89]

身份材料、AlreadyTerminal 推进及适配器规则以对应规范表为唯一可执行定义；
字段或分支中的 `IdentityRule::PreparedPushIntent`、`CompletionRule::AlreadyTerminal`
是受校验的规则引用，不是自由解释的说明。表内 snake_case 值和类型/枚举均为合同
标识；其余叙述、标题及表头使用中文。[Q:61] [Q:66] [Q:78] [Q:87] [Q:89]

## 类型：RunContext（PROPOSED）

创建者：RunContextFactory（scheduler/event/manual 适配器）。消费者：prepare、project、coordinator、finalizer。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:16] [Q:33] [Q:40] [Q:74] [Q:79] [unit:MU-p01] [producer:p01-scheduled]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| schema_version | u32 | 固定为 1；未知 schema 必须拒绝 | 纳入 |
| run_id | RunId | 首次准备身份；重放和重启读取原值 | 纳入 |
| unit_id | UnitId | 必须存在于目录，且生产者属于此 Unit | 纳入 |
| namespace | Namespace | 仅限 Production 或 Test(run_id)，禁止跨命名空间混用 | 纳入 |
| business_date | Date | 由交易日 authority 一次性捕获 | 纳入 |
| calendar_date | Date | 捕获的本地自然日，不得替代 business_date | 纳入 |
| phase | PhaseEpic | 仅限 Preopen、Auction、Intraday、Postclose，不是完成所有者身份 | 纳入 |
| trigger | Trigger | 仅限 Scheduled(schedule_id)、Event(producer_id,source_ref)、Manual(command_id,authenticated_operator_ref) | 纳入 |
| occurrence | OccurrenceId | 来自注册生产者家族的规范化业务 occurrence | 纳入 |
| captured_business_time | UtcMicros | 一次性捕获的业务时钟，不是 shadow 执行时钟 | 纳入 |
| activation_generation | u64 | 已批准 Unit 完成所有者栅栏的 generation | 纳入 |
| build_commit | GitSha40 | 必须与 activation 校验的构建身份一致 | 纳入 |
| catalog_sha256 | Sha256 | 必须是解析 Unit/完成所有者所使用的目录 | 纳入 |
| source_contract_version | NonEmptyText | 已批准来源合同版本，不得从 payload 猜测 | 纳入 |
| template_version | NonEmptyText | 本 occurrence 首次渲染时冻结的版本 | 纳入 |

## 类型：PreparedFacts（PROPOSED）

创建者：prepare（经过准入的来源适配器与不可变构造器）。消费者：active 和 shadow 的 project。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:33] [Q:40] [Q:72] [Q:83] [unit:MU-news-ai] [producer:news-ai-same-tick]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| run_context_sha256 | Sha256 | 绑定已捕获 RunContext 的规范化字节 | 纳入 |
| source_contract_id | NonEmptyText | 标识生产者的来源合同 | 纳入 |
| source_contract_version | NonEmptyText | 必须等于 RunContext 的来源合同版本 | 纳入 |
| source_refs | Vec<SourceRef> | provider、external_id、source_contract、content_sha256 引用有序且唯一，捕获前冻结顺序 | 纳入 |
| canonical_facts | ExactBytes | 来源事实须通过固定字段 schema 校验，不是渲染文本 | 外部原始字节 |
| facts_sha256 | Sha256 | SHA256(canonical_facts)，不得换用后续 provider 批次重算 | 派生且排除自身 |
| provider_observed_at | Vec<SourceTime> | 每个来源有 source_ref_id 与 observed_at/as_of UtcMicros；未知时间显式为 None，不能写当前时间 | 纳入 |
| verified_empty | bool | 仅在限定范围来源验证成功且确为空时为 true，失败不能当空结果 | 纳入 |
| model_output_refs | Vec<ModelOutputRef> | model/version/input_sha256/output_sha256/protected_ref 按序捕获一次，未使用模型时为空 | 纳入 |

## 类型：SemanticProjection（PROPOSED）

创建者：纯函数 project(PreparedFacts)。消费者：shadow 比较器与 PreparedPush 构造器。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:19] [Q:40] [Q:72] [Q:83] [unit:MU-p01] [producer:p01-compensation]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| audience | AudienceId | 显式不可变路由受众，不得从日志猜测 | 纳入 |
| monitor_kind | Option<MonitorKind> | 冻结的 65 个 monitor 枚举之一；仅目录中枚举外生产者允许 None | 纳入 |
| sub_kind | SubKind | None 或经批准的类型特定值，不是另一物理完成所有者 | 纳入 |
| occurrence | OccurrenceId | 等于 RunContext 的 occurrence | 纳入 |
| business_subject | SubjectId | 类型化 Global 或规范代码/业务对象，不得解析显示文本反推 | 纳入 |
| severity | Severity | 仅限 Emergency、Important、Info、Research；严重性不能授权 Uncertain 重发 | 纳入 |
| suppression | Suppression | 仅限 Eligible 或 Suppressed(reason,eligible_after) | 纳入 |
| completion_policy_id | NonEmptyText | 目录绑定的注册策略身份 | 纳入 |
| completion_policy_version | NonEmptyText | shadow 和 active 使用同一版本 | 纳入 |
| evidence_fingerprint | Sha256 | SHA256(有序来源引用及已捕获模型引用) | 纳入 |
| template_id | NonEmptyText | 注册渲染器身份 | 纳入 |
| template_version | NonEmptyText | 等于 RunContext.template_version | 纳入 |
| canonical_bytes | ExactBytes | 仅编码前述语义字段 | 派生且排除自身 |
| sha256 | Sha256 | SHA256(canonical_bytes) | 派生且排除自身 |

## 类型：PreparedPush（PROPOSED）

创建者：纯 project 与首次渲染后的 Ready 构造器。消费者：业务 intent 存储与 DeliveryCoordinator。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:10] [Q:72] [Q:76] [Q:89] [unit:MU-p01] [evidence:counted-envelope]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| intent_id | IntentId | IdentityRule::PreparedPushIntent | 纳入 |
| decision_id | DecisionId | intent_id 的稳定 authority 域派生值，跨 attempt/重启保持一致 | 纳入 |
| unit_id | UnitId | 等于捕获的 RunContext 及注册完成所有者 | 纳入 |
| occurrence | OccurrenceId | 等于 RunContext 和 SemanticProjection 的 occurrence | 纳入 |
| subject | SubjectId | 等于 SemanticProjection.business_subject | 纳入 |
| run_context_sha256 | Sha256 | 原始 RunContext 的规范化绑定 | 纳入 |
| prepared_facts_sha256 | Sha256 | PreparedFacts 外层规范化绑定，包含事实内容摘要 | 纳入 |
| semantic_projection_sha256 | Sha256 | 等于 SemanticProjection.sha256 | 纳入 |
| source_binding | SourceBinding | 来源合同 ID/version、有序来源引用及证据指纹 | 纳入 |
| rendered_bytes | ExactBytes | 首次渲染的 UTF-8 原始字节，保留有意空白，重放不得再次渲染 | 外部原始字节 |
| rendered_sha256 | Sha256 | SHA256(rendered_bytes)；同一 intent 漂移必须进入 ResolutionRequired | 纳入 |

## 类型：VerifiedTerminalRef（PROPOSED）

创建者：私有 authority 适配器的重新查询和精确绑定校验。消费者：DeliveryResult 与重新查询/校验的 finalizer。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:7] [Q:46] [Q:55] [Q:78] [Q:87] [evidence:startup-kind-map] [unit:MU-p01] [unit:MU-news-flash-aggregate]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| ref_id | TerminalRefId | 稳定 authority 处置身份，不是日志 ID | 纳入 |
| authority_class | AuthorityClass | 仅限通过一致性测试的 GenericCounted、P01Dedicated、N02Dedicated | 纳入 |
| namespace | Namespace | 与原 intent 命名空间及受众范围精确一致 | 纳入 |
| decision_id | DecisionId | 精确绑定已持久化的不可变 decision | 纳入 |
| attempt_id | Option<AttemptId> | 绑定具体 attempt；仅经校验的尝试前拒绝或人工处置可为 None | 纳入 |
| intent_id | IntentId | 精确绑定请求的业务 intent | 纳入 |
| unit_id | UnitId | 与请求 intent 的 Unit 和完成所有者绑定一致 | 纳入 |
| occurrence | OccurrenceId | 精确绑定首次业务 occurrence，不是当前 tick | 纳入 |
| business_date | Date | 精确绑定捕获的业务日 | 纳入 |
| subject | SubjectId | 精确绑定业务对象 | 纳入 |
| audience | AudienceId | 精确绑定目标受众 | 纳入 |
| template_id | NonEmptyText | 精确绑定已持久化模板身份，不能仅比较版本标签 | 纳入 |
| template_version | NonEmptyText | 精确绑定已持久化模板版本 | 纳入 |
| rendered_sha256 | Sha256 | 精确绑定已持久化渲染字节哈希 | 纳入 |
| terminal_disposition | TerminalDisposition | 仅限 Accepted、Rejected、Uncertain、ManualConfirmedAccepted、ManualConfirmedNotDelivered；人工接受不能变成 TransportAccepted | 纳入 |
| evidence_sha256 | Sha256 | 已校验回执/处置或经过认证的人工处置证据哈希，不复制回执内容 | 纳入 |
| durable_schema_version | NonEmptyText | authority 兼容的持久化 schema 版本 | 纳入 |
| verified_at | UtcMicros | 重新查询的时间仅供审计，不进入稳定绑定摘要或 shadow 语义 | 派生且排除自身 |
| binding_sha256 | Sha256 | IdentityRule::TerminalBinding | 派生且排除自身 |

## 类型：CompatibilityEvidenceRef（PROPOSED）

创建者：NotificationService 兼容适配器。消费者：仅 CLI/报告观察者，不是权威 finalizer。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:4] [Q:7] [Q:44] [unit:MU-cli-single] [producer:cli-single-default]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| compat_id | CompatId | 本地调用/结果身份，不是 durable 回执 | 纳入 |
| intent_id | IntentId | 报告调用意图，不得打开生产 durable 数据库 | 纳入 |
| unit_id | UnitId | 目录中已有 CLI/兼容路径 Unit | 纳入 |
| occurrence | OccurrenceId | 同一报告调用或 occurrence | 纳入 |
| configured_channels | Vec<ChannelId> | 调用时配置的渠道有序且唯一 | 纳入 |
| attempted_channels | Vec<ChannelId> | 实际尝试渠道必须是配置渠道的子集 | 纳入 |
| weak_outcomes | Vec<WeakOutcome> | 每个已尝试渠道记录 Accepted、Rejected 或 Unknown 及本地证据引用，不声明权威性 | 纳入 |
| local_evidence_sha256 | Sha256 | 本地逐渠道事实哈希，不是远端回执哈希 | 纳入 |
| observed_at | UtcMicros | 本地观察时间，不是远端 accepted_at | 纳入 |
| not_authoritative | TrueLiteral | 固定为 true，禁止转换成 VerifiedTerminalRef | 纳入 |

## 类型：CompletionPolicy（PROPOSED）

创建者：版本化目录策略注册表，不允许调用方自行定义策略。消费者：project 提案与已注册 finalizer。
上述逐字段生命周期和规范化规则适用于**每一行**。
[Q:2] [Q:9] [Q:73] [Q:85] [Q:86] [Q:87] [Q:88] [unit:MU-p01] [unit:MU-cli-single]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| id | NonEmptyText | 稳定的注册策略 ID | 纳入 |
| version | NonEmptyText | 冻结版本，修改须经新的合同批准 | 纳入 |
| completion_owner | CatalogOwnerRef | Unit、catalog 的 SHA 与精确完成所有者身份，不是 PushKind 或传输额度 | 纳入 |
| advance_event | AdvanceEvent | 仅限 AcceptedBound 或 AcceptedOrManualBound，不能依据 bool/Ok/本地审计 | 纳入 |
| schedule_close_policy | ScheduleClosePolicy | 仅限 OnAccepted、VerifiedNoData、ExplicitDisabled、SuppressedOccurrence 中已选分支 | 纳入 |
| notification_cursor_policy | CursorPolicy | 仅限 AcceptedBoundOnly 或 Never；关闭 schedule 不意味着推进通知游标 | 纳入 |
| no_data_policy | NoDataPolicy | 仅限 KeepOpen 或 CloseVerifiedOccurrence，后者必须绑定 verified_empty 事实 | 纳入 |
| disabled_policy | DisabledPolicy | 仅限 KeepOpen 或 CloseDisabledOccurrence，不能激活禁用生产者 | 纳入 |
| retry_policy | RetryPolicy | 仅限 Never、InputBackoff(not_before)、AuthorizedRejected(not_before,max_attempts)，仍受 disposition/栅栏约束 | 纳入 |
| uncertain_manual_policy | UncertainPolicy | QuarantineThenVerifiedManual；任何严重性都不得盲目重发 | 纳入 |
| already_terminal_policy | AlreadyTerminalPolicy | CompletionRule::AlreadyTerminal | 纳入 |
| allowed_authority | Vec<AuthorityClass> | 必须显式列出通过一致性验证的 authority；COMPAT 不属于 authority 类别 | 纳入 |
| finalizer_kind | FinalizerKind | 仅限 BoundCursor、ScheduleOnly、CompatibilityObservation；仅注册完成所有者可以执行 | 纳入 |
| retention_class | RetentionClass | 采用迁移/监管/模型/交易类别中最严格策略，非终态不得自动清理 | 纳入 |

## 类型：JobDecision（PROPOSED）

创建者：纯函数 project(PreparedFacts)。消费者：应用调度器和 CompletionPolicy 提案构造器。
规范化材料为 variant 标签及全部类型化 payload，嵌套对象使用前述绑定；诊断文字不参与。
ReasonCode 必须来自注册表；Option<UtcMicros> 只表示捕获的资格时间，不是盲目重试许可。
任何分支都不能调用 provider/LLM/send 或直接推进游标。[Q:33] [Q:34] [Q:72] [Q:85]
[Q:86] [unit:MU-news-ai] [producer:news-ai-same-tick]

| 分支 | 载荷 | 允许输入 | 业务提案 | 禁止行为 | 依据 |
| --- | --- | --- | --- | --- | --- |
| Ready | PreparedPush | 已验证的捕获事实与已准入策略 | 持久化不可变 intent，由 coordinator 调度 | 直接发送或提前完成 | [Q:72] |
| NoData | {reason:ReasonCode,evidence_sha256:Sha256} | 已成功验证为空且绑定 PreparedFacts 的来源 | 仅允许策略选择的 schedule 关闭提案 | 把来源失败当空结果，或推进通知游标 | [Q:85] [Q:86] |
| Disabled | {reason:ReasonCode} | 显式禁用策略/activation 快照 | 按策略保持开放或关闭禁用 occurrence | 激活生产者或宣称已接受 | [Q:21] [Q:30] |
| BlockedOnInput | {reason:ReasonCode,retry_after:Option<UtcMicros>} | 来源缺失、未就绪或证据无效 | 保持 occurrence 待处理并隔离生产者 | 伪造事实或业务完成 | [Q:12] |
| Suppressed | {reason:ReasonCode,eligible_after:Option<UtcMicros>} | 捕获的抑制/冷却策略 | 保持开放或显式关闭受抑制 schedule | 从抑制推断已投递 | [Q:19] [Q:86] |
| RetryableFailure | {reason:ReasonCode,retry_after:Option<UtcMicros>} | 已分类的发送前可恢复失败 | 按策略进行有界准备重试 | 把发送后 Uncertain 改成发送前重试 | [Q:9] [Q:85] |
| PermanentFailure | {reason:ReasonCode} | 已分类的不可恢复准备/合同失败 | 停止并暴露类型化失败 | 静默丢弃并标记 Completed | [Q:12] [Q:101] |

## 类型：DeliveryResult（PROPOSED）

创建者：coordinator 的权威校验适配器或隔离的兼容适配器。消费者：应用观察者及注册
CompletionPolicy/finalizer。规范化材料为 variant 与类型化引用绑定（或 ReasonCode），
不复制回执。只有私有 authority 查询能创建 VerifiedTerminalRef，finalizer 使用前再次
查询和校验；CompatibilityEvidenceRef 没有转换到该类型的接口。[Q:7] [Q:27] [Q:46]
[Q:78] [Q:87] [unit:MU-cli-single] [producer:cli-single-default]

表内 authority 合同值为 strong、compat、none；推进合同值为 policy_bound、never。
policy_bound 不是无条件推进许可，必须满足终态完成合同。[Q:2] [Q:78] [Q:87]

| 分支 | 载荷 | 权威类别 | 权威完成推进 | 条件 | 依据 |
| --- | --- | --- | --- | --- | --- |
| TransportAccepted | VerifiedTerminalRef | strong | policy_bound | 重新验证 Accepted 处置及精确绑定回执，不能来自人工接受或本地审计 | [Q:2] [Q:7] |
| TransportRejected | VerifiedTerminalRef | strong | never | 重新验证 Rejected，只有当前显式重试授权才可发起新 attempt | [Q:9] [Q:85] |
| TransportUncertain | VerifiedTerminalRef | strong | never | 重新验证 Uncertain，进入隔离/人工处置，不得盲目重发 | [Q:9] |
| AlreadyTerminal | VerifiedTerminalRef | strong | policy_bound | CompletionRule::AlreadyTerminal | [Q:46] [Q:87] |
| BestEffortAccepted | CompatibilityEvidenceRef | compat | never | 所有已配置渠道均实际尝试且弱接受，不声明远端权威性 | [Q:4] [Q:7] |
| PartiallyAccepted | CompatibilityEvidenceRef | compat | never | 至少一个渠道弱接受，且至少一个已配置渠道未接受 | [Q:4] [Q:7] |
| NoChannelConfigured | ReasonCode | compat | never | 配置渠道为空，返回 transport.no_channel_configured | [Q:7] [Q:101] |
| AllChannelsFailed | CompatibilityEvidenceRef | compat | never | 配置非空但无渠道弱接受，Unknown 必须保持未知 | [Q:7] |
| Blocked | ReasonCode | none | never | authority 查询、策略、lease 或绑定失败，不是传输尝试回执 | [Q:78] [Q:101] |

COMPAT 明确为 not_authoritative：禁止构造 VerifiedTerminalRef、投影为 TransportAccepted、
打开生产 durable 数据库或推进权威通知完成。弱渠道 Unknown 不能按确定拒绝重试。
业务 Completed/NoData/Disabled 不属于 DeliveryResult；ManualConfirmedAccepted 保留为
AlreadyTerminal 中独立处置，使用独立证据与指标，不能变成 TransportAccepted。
[Q:4] [Q:7] [Q:46] [Q:87] [unit:MU-cli-summary] [producer:cli-summary-default]

## 业务完成分支（PROPOSED）

仅 finalizer 可以把提案应用到注册完成所有者；应用调用方不得直接提交。这是策略合同，
不是已经实现的 SQL 状态机。ScheduleOnly/CompatibilityObservation 不推进通知游标；
BoundCursor 必须同时满足允许的 authority、精确绑定和 AcceptedBoundOnly。
AlreadyTerminal 的绑定、处置和提案由下方终态完成合同逐行定义，不能只看 variant 名称。
[Q:2] [Q:73] [Q:78] [Q:85] [Q:86] [Q:87] [unit:MU-p01] [unit:MU-cli-single]

| 输入 | 允许策略 | 时段提案 | 游标提案 | 禁止行为 | 依据 |
| --- | --- | --- | --- | --- | --- |
| Ready | 任一已注册策略 | 收到投递结果前保持 KeepOpen | None | 持久化 intent 后提前完成 | [Q:2] |
| NoData | KeepOpen 或 CloseVerifiedOccurrence | 仅凭 verified_empty 证据 KeepOpen 或 Close | None | 关闭未经验证为空的来源 | [Q:85] [Q:86] |
| Disabled | KeepOpen 或 CloseDisabledOccurrence | KeepOpen 或显式关闭禁用 occurrence | None | 激活 INACTIVE/STARVED/OPT-IN | [Q:21] [Q:30] |
| BlockedOnInput | InputBackoff 或 Never | KeepOpen | None | 把来源失败隐藏为 NoData | [Q:12] |
| Suppressed | SuppressedOccurrence 或 KeepOpen | 显式抑制关闭或 KeepOpen | None | 把受抑制当已投递 | [Q:19] [Q:86] |
| RetryableFailure | InputBackoff 或 Never | 保持 KeepOpen 并遵守 not_before | None | 在此重试发送后的 Uncertain | [Q:9] |
| PermanentFailure | Never | 保持 KeepOpen 并记录类型化停止原因 | None | 静默成功 | [Q:101] |
| TransportAccepted | AcceptedBound 或 AcceptedOrManualBound | OnAccepted 可关闭 | BoundCursor 可 AdvanceAccepted；Never 仍为 None | 完成所有者或绑定不匹配 | [Q:2] [Q:78] |
| TransportRejected | AuthorizedRejected 或 Never | KeepOpen；遵守显式 durable 重试授权和 max_attempts | None | 未经授权重试 | [Q:85] |
| TransportUncertain | QuarantineThenVerifiedManual | KeepOpen 并保留证据 | None | 任何盲目重发 | [Q:9] [Q:88] |
| AlreadyTerminal | CompletionRule::AlreadyTerminal | CompletionRule::AlreadyTerminal.schedule | CompletionRule::AlreadyTerminal.cursor | CompletionRule::AlreadyTerminal.forbidden | [Q:46] [Q:87] |
| BestEffortAccepted | CompatibilityObservation | 仅记录本地观察 | None | 转换成 VerifiedTerminalRef | [Q:7] |
| PartiallyAccepted | CompatibilityObservation | 仅记录本地逐渠道观察 | None | 隐藏失败或未知渠道 | [Q:7] |
| NoChannelConfigured | CompatibilityObservation | 仅记录本地无渠道观察 | None | 计为已尝试或已投递 | [Q:7] |
| AllChannelsFailed | CompatibilityObservation | 仅记录本地失败观察 | None | 把 Unknown 按确定失败重试 | [Q:7] [Q:9] |
| Blocked | Never 或有界发送前 InputBackoff | KeepOpen 并保留原因 | None | 绕过 coordinator 或绑定门禁 | [Q:78] |

非终态事实保留到完成处置；终态迁移证据至少保留 90 天，更严格的监管/模型/交易保留期
优先。这是策略字段合同，不是清理功能已经实现的声明。[Q:48] [Q:88]

## 身份合同（PROPOSED）

本表的材料列是按序字段集合，排除列是禁止参与该身份摘要的字段集合；SHA256CanonicalTuple
使用前述 v1 域标签与规范化元组编码。IdentityRule::PreparedPushIntent 精确引用首行，
TerminalBinding 定义 VerifiedTerminalRef.binding_sha256，不能按展示文本另造身份。
同一 intent 的不可变材料漂移不能改用新 payload 哈希生成新身份来绕过处置。
[Q:16] [Q:59] [Q:78] [Q:87] [Q:89] [unit:MU-p01] [evidence:counted-envelope]

| 规则 | 函数 | 有序材料 | 排除材料 | 冲突处置 | 依据 |
| --- | --- | --- | --- | --- | --- |
| PreparedPushIntent | SHA256CanonicalTuple | namespace,unit_id,completion_owner,source_contract_id,occurrence,subject,audience | payload_sha256,rendered_sha256,evidence_sha256 | ResolutionRequired | [Q:16] [Q:89] |
| TerminalBinding | SHA256CanonicalTuple | ref_id,authority_class,namespace,decision_id,attempt_id,intent_id,unit_id,occurrence,business_date,subject,audience,template_id,template_version,rendered_sha256,terminal_disposition,evidence_sha256,durable_schema_version | verified_at,binding_sha256 | Blocked | [Q:78] [Q:87] |

## 终态完成合同（PROPOSED）

CompletionRule::AlreadyTerminal 精确引用本表。RequeryExactBinding 表示重新查询指定
authority，并逐项验证身份合同中 TerminalBinding 的全部绑定材料、其摘要以及当前
intent 的预期值；不得仅比较处置标签。任何绑定失败都返回 Blocked，不产生表内提案。
表内“+”表示条件必须同时成立；人工接受和传输接受分别计量。非 BoundCursor 或
CursorPolicy::Never 不推进通知游标，是否关闭 schedule 仍由注册策略独立选择。
[Q:2] [Q:46] [Q:78] [Q:85] [Q:86] [Q:87] [unit:MU-p01]

| 处置 | 绑定校验 | 游标必要策略 | 时段提案 | 游标提案 | 禁止行为 | 依据 |
| --- | --- | --- | --- | --- | --- | --- |
| Accepted | RequeryExactBinding | AllowedAuthority+BoundCursor+AcceptedBoundOnly | OnAccepted | AdvanceAccepted | NeverInferFromVariant | [Q:2] [Q:78] [Q:87] |
| ManualConfirmedAccepted | RequeryExactBinding | AllowedAuthority+BoundCursor+AcceptedBoundOnly+AcceptedOrManualBound | OnAccepted | AdvanceManualAccepted | NeverTransportAccepted | [Q:46] [Q:87] |
| Rejected | RequeryExactBinding | RegisteredPolicy | KeepOpen | None | NeverAdvanceOrBlindRetry | [Q:85] [Q:87] |
| Uncertain | RequeryExactBinding | QuarantineThenVerifiedManual | KeepOpen | None | NeverAdvanceOrBlindRetry | [Q:9] [Q:87] |
| ManualConfirmedNotDelivered | RequeryExactBinding | RegisteredPolicy | KeepOpen | None | NeverAdvance | [Q:46] [Q:87] |

## monitor 到 durable 的映射（CURRENT）

源码依据为冻结基线的
`src/bin/monitor/durable_delivery_runtime.rs::durable_kind_and_sub_kind_with_override`
及 `src/durable_delivery/model.rs::PushKind`。26 个 monitor 枚举映射到 23 个 durable 枚举，
DailyReport 保留 FactorIC/SectorTier/CapitalVerify 子类型。映射存在不代表存在生产者、
激活状态、回执或 Unit；已存 decision 的恢复不能激活普通 INACTIVE 载入路径。
[Q:21] [Q:30] [Q:75] [evidence:startup-kind-map] [evidence:startup-resume] [evidence:push-kind]

| monitor类型 | durable类型 | 子类型 | 依据 |
| --- | --- | --- | --- |
| HoldingPlan | HoldingPlan | None | [evidence:startup-kind-map] |
| HoldingEvent | HoldingEvent | None | [evidence:startup-kind-map] |
| T0Advice | T0Advice | None | [evidence:startup-kind-map] |
| CandidateTriggered | CandidateTriggered | None | [evidence:startup-kind-map] |
| PreopenNewsHot | PreopenNewsHot | None | [evidence:startup-kind-map] |
| CloseCall | CloseCall | None | [evidence:startup-kind-map] |
| ForbiddenOps | ForbiddenOps | None | [evidence:startup-kind-map] |
| PaperTrade | PaperTrade | None | [evidence:startup-kind-map] |
| ReviewMarket | ReviewMarket | None | [evidence:startup-kind-map] |
| ReviewLhb | ReviewLhb | None | [evidence:startup-kind-map] |
| ReviewSignal | ReviewSignal | None | [evidence:startup-kind-map] |
| ReviewFailure | ReviewFailure | None | [evidence:startup-kind-map] |
| TomorrowWatch | TomorrowWatch | None | [evidence:startup-kind-map] |
| EventCalendar | EventCalendar | None | [evidence:startup-kind-map] |
| ReviewProviderTopN | ReviewProviderTopN | None | [evidence:startup-kind-map] |
| SectorTop | SectorTop | None | [evidence:startup-kind-map] |
| SectorAnomaly | SectorAnomaly | None | [evidence:startup-kind-map] |
| IndustryChain | IndustryChain | None | [evidence:startup-kind-map] |
| PositionReview | PositionReview | None | [evidence:startup-kind-map] |
| ReviewBacktest | ReviewBacktest | None | [evidence:startup-kind-map] |
| WatchlistTracking | WatchlistTracking | None | [evidence:startup-kind-map] |
| CatalystReview | CatalystReview | None | [evidence:startup-kind-map] |
| FactorIC | DailyReport | FactorIC | [evidence:startup-kind-map] |
| SectorTier | DailyReport | SectorTier | [evidence:startup-kind-map] |
| CapitalVerify | DailyReport | CapitalVerify | [evidence:startup-kind-map] |
| DailyReport | DailyReport | 按请求选择 FactorIC/SectorTier/CapitalVerify 或 None | [evidence:startup-kind-map] |

## 未直接映射的 monitor 类型（CURRENT 状态；PROPOSED 处置）

下列 39 类没有直接通用持久计数映射。adapt_or_conform 表示已有较强专用协议时
保留其 authority，再补共享应用/intent/finalizer 边界，不是重写所有 authority。
retain_starved/retain_opt_in 不授权恢复或启用输入；keep_inactive 不创建 scheduler
或生产者。枚举外 CLI 路径已计入 102-producer/52-Unit 目录，不参与 65-kind 减法。
[Q:21] [Q:27] [Q:28] [Q:30] [Q:93] [unit:MU-news-flash-aggregate]
[producer:news-flash-aggregate] [unit:MU-cli-single]

| monitor类型 | 状态 | 处置 | 依据 |
| --- | --- | --- | --- |
| Announcement | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:news-announcement] |
| AuctionVolume | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:auction-volume] |
| VirtualWatch | STARVED | retain_starved | [evidence:monitor-loop] [producer:virtual-watch-pilot] |
| LimitBoards | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:limit-boards-first] |
| FundInflow | INACTIVE | keep_inactive | [evidence:push-kind] |
| AuctionRepush | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:auction-repush] |
| WeeklySOP | INACTIVE | keep_inactive | [evidence:push-kind] |
| StockPick | INACTIVE | keep_inactive | [evidence:push-kind] |
| TurnoverTop | INACTIVE | keep_inactive | [evidence:push-kind] |
| CandidateBoard | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:candidate-board] |
| NewsRanked | INACTIVE | keep_inactive | [evidence:dispatch-disabled] |
| AccountMode | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:account-mode-main] |
| DataMode | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:data-mode] |
| PaperSell | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:paper-sell-intraday] |
| SnapshotStale | ACTIVE | adapt_or_conform | [evidence:monitor-main] [producer:snapshot-stale-startup] |
| AttributionDaily | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:attribution-daily] |
| G5bAttribution | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:g5b-attribution] |
| IntradayMarket | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:market-view-periodic] |
| NewsCatalyst | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:catalyst-announcement] |
| NewsToIdea | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:d01-announcement] |
| IndustryChainIntraday | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:industry-chain-periodic] |
| PostFixedPriceOrder | STARVED | retain_starved | [evidence:monitor-loop] [producer:post-fixed-order] |
| PostFixedPriceFill | STARVED | retain_starved | [evidence:monitor-loop] [producer:post-fixed-fill] |
| StPriceLimitChanged | ACTIVE | adapt_or_conform | [evidence:monitor-loop] [producer:st-price-limit-batch] |
| EtfClosingCallAuction | INACTIVE | keep_inactive | [evidence:etf-unused] |
| BlockTradeIntradayConfirm | ACTIVE | adapt_or_conform | [evidence:review-batch] [producer:block-confirm-side-route] |
| BlockTradePriceRange | INACTIVE | keep_inactive | [evidence:block-review] |
| PaperReview | STARVED | retain_starved | [evidence:monitor-loop] [producer:paper-review-noon] |
| CandidateInvalidated | ACTIVE | adapt_or_conform | [evidence:candidate-board] [producer:candidate-invalidated] |
| IpoListingApproval | INACTIVE | keep_inactive | [evidence:review-manual] |
| IpoProspectus | INACTIVE | keep_inactive | [evidence:review-manual] |
| IpoCatalyst | ACTIVE | adapt_or_conform | [evidence:review-batch] [producer:ipo-catalyst-side-route] |
| PolicyHit | INACTIVE | keep_inactive | [evidence:policy-classify] |
| EarningsBeat | OPT-IN | retain_opt_in | [evidence:news-loop] [producer:earnings-beat] |
| EarningsMiss | OPT-IN | retain_opt_in | [evidence:news-loop] [producer:earnings-miss] |
| AnalystUpgrade | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:analyst-upgrade] |
| MarketActionAlert | ACTIVE | adapt_or_conform | [evidence:account-push] [producer:account-frozen-side] |
| NewsFlashCritical | INACTIVE | keep_inactive | [evidence:flash-reserve] |
| NewsFlashAggregated | ACTIVE | adapt_or_conform | [evidence:news-loop] [producer:news-flash-aggregate] |

## durable 状态（CURRENT）与应用投影（PROPOSED）

状态名来自冻结基线 `src/durable_delivery/model.rs::DecisionState` 第 800–814 行的
14 个枚举。应用投影是新提案，不表示 Rust 当前已返回这些 RFC 类型。“终态”指已封存
传输处置检查点，不等于业务通知已接受；Uncertain 在业务层仍未解决。审计/任务转换
待完成状态返回 Blocked，直到 authority 能提供可验证终态引用。不得从状态名、
Ok 或日志提升事实权限；业务 intent 状态不能复制 durable 14 态。
[Q:7] [Q:75] [Q:78] [evidence:startup-reconcile] [evidence:startup-list-deliverable]
[evidence:startup-begin-attempt]

| 状态 | 应用结果 | 传输处置终态 | 自动发送重试 | 业务最终化 | 依据 |
| --- | --- | --- | --- | --- | --- |
| Reserved | Blocked | 否 | lease_fenced_first_attempt | 否 | [Q:90] |
| AttemptInFlight | Blocked | 否 | never_until_reconciled | 否 | [Q:9] |
| AcceptedAuditPending | Blocked | 否 | never | after_authority_sealed | [Q:78] |
| AcceptedTaskTransitionPending | Blocked | 否 | never | after_authority_sealed | [Q:78] |
| Delivered | TransportAccepted/AlreadyTerminal | 是 | never | accepted_binding_only | [Q:2] [Q:87] |
| RejectedAuditPending | Blocked | 否 | never_until_reconciled | 否 | [Q:78] |
| RejectedTaskTransitionPending | Blocked | 否 | never_until_reconciled | 否 | [Q:78] |
| RejectedDurable | TransportRejected/AlreadyTerminal | 是 | explicit_authorization_only | rejection_proposal_no_cursor | [Q:85] |
| UncertainAuditPending | Blocked | 否 | never | 否 | [Q:9] |
| UncertainTaskTransitionPending | Blocked | 否 | never | 否 | [Q:9] |
| UncertainManualReview | TransportUncertain/AlreadyTerminal | 是 | never | quarantine_no_cursor | [Q:9] [Q:87] |
| ManualRejectedAuditPending | Blocked | 否 | never | 否 | [Q:46] |
| ManualRejectedTaskTransitionPending | Blocked | 否 | never | 否 | [Q:46] |
| ManualResolvedRejected | AlreadyTerminal | 是 | never | manual_not_delivered_no_cursor | [Q:46] [Q:87] |

## 类型：ReasonCode（PROPOSED）

创建者：检测到条件的类型化边界。消费者：project、coordinator、finalizer、
operator/告警适配器及测试。规范化材料是精确 ASCII 代码，不是解释文字。最低注册表
使用稳定小写 namespace.suffix，新增项需要已批准依据和 schema 复核。
解释修改不能驱动控制流；下表重试仅表示资格，仍受栅栏和权威授权约束。
[Q:9] [Q:85] [Q:101] [unit:MU-p01] [producer:p01-scheduled]

| 代码 | 条件 | 处理 | 依据 |
| --- | --- | --- | --- |
| schedule.not_trading_day | 交易日 authority 确认为非交易日 | 按策略产生 NoData/Disabled，不发送 | [Q:101] [Q:28] |
| schedule.window_not_open | 捕获业务时间早于有效窗口 | 保持 occurrence 待处理，直到具备资格 | [Q:101] [Q:28] |
| schedule.window_expired | 捕获业务时间超过补偿窗口 | 仅产生策略允许的 schedule 提案，不暗示投递 | [Q:101] [Q:28] |
| schedule.occurrence_closed | 精确调度 occurrence 已关闭 | 不产生新调度，通知状态仍独立 | [Q:101] [Q:86] |
| input.source_unavailable | provider 读取失败 | BlockedOnInput，仅允许有界发送前重试 | [Q:101] [Q:12] |
| input.source_unready | 所需生产者能力未就绪 | BlockedOnInput，并隔离生产者 | [Q:101] [Q:12] |
| input.evidence_invalid | 来源引用/哈希/时间绑定无效 | BlockedOnInput，不伪造事实 | [Q:101] [Q:59] |
| input.no_verified_batch | 没有经过准入的同 tick 批次 | BlockedOnInput，NewsAI 不混用跨批次事实 | [Q:101] [Q:33] |
| input.account_snapshot_missing | 所需账户快照缺失 | BlockedOnInput，不代入其他账户或组合 | [Q:101] [Q:12] |
| input.namespace_violation | 来源或 intent 命名空间不一致 | PermanentFailure，禁止跨命名空间使用 | [Q:101] [Q:49] |
| policy.disabled | 生产者或 activation 明确禁用 | Disabled，保持注册策略 | [Q:101] [Q:21] |
| policy.starved | 目录来源仍为 STARVED | BlockedOnInput，不擅自激活产品 | [Q:101] [Q:30] |
| policy.opt_in_disabled | 所需 opt-in 尚未批准 | Disabled，不擅自激活产品 | [Q:101] [Q:30] |
| policy.cooldown_active | 捕获的冷却策略排除当前 occurrence | Suppressed 直到具备资格，不算已投递 | [Q:101] [Q:19] |
| policy.daily_budget_full | 适用的共享预算已耗尽 | Suppressed，额度不是完成所有者 | [Q:101] [Q:16] |
| policy.suppressed | 显式语义抑制规则命中 | Suppressed 提案，不推进通知游标 | [Q:101] [Q:19] |
| intent.payload_conflict | 同一身份的不可变 payload/evidence 哈希不同 | ResolutionRequired，不覆盖或重发 | [Q:101] [Q:89] |
| intent.expected_version_conflict | 业务 CAS 版本不一致 | ResolutionRequired，阻断晋级 | [Q:101] [Q:77] |
| intent.lease_held | 其他完成所有者的 lease 尚未过期 | Blocked，不发起竞争 attempt | [Q:101] [Q:90] |
| intent.transition_conflict | intent 与 journal 转换不一致 | ResolutionRequired，不伪造完成 | [Q:101] [Q:97] |
| transport.rejected | 经过验证的 authority 拒绝 attempt | TransportRejected，仅显式授权可重试 | [Q:101] [Q:85] |
| transport.uncertain | 经过验证的 authority 记录未知结果 | TransportUncertain，隔离且不盲目重试 | [Q:101] [Q:9] |
| transport.no_channel_configured | 未配置兼容渠道 | NoChannelConfigured，无尝试或回执 | [Q:101] [Q:7] |
| transport.all_channels_failed | 没有弱渠道被接受 | AllChannelsFailed，保留 Unknown，不提升权威性 | [Q:101] [Q:7] |
| transport.partially_accepted | 仅部分弱渠道被接受 | PartiallyAccepted，不提升权威性 | [Q:101] [Q:7] |
| finalizer.terminal_ref_invalid | authority 重新查询不能验证引用 | Blocked，不修改业务事实 | [Q:101] [Q:78] |
| finalizer.binding_mismatch | 引用不能绑定请求 intent | Blocked，不推进完成 | [Q:101] [Q:87] |
| finalizer.cas_conflict | 完成所有者版本已变化 | ResolutionRequired，不覆盖 | [Q:101] [Q:77] |
| finalizer.deadline_exceeded | Accepted 到 Finalized 超过两个周期目标或五分钟硬上限 | 暴露延迟，达到硬上限时阻断晋级 | [Q:101] [Q:38] |
| finalizer.transition_append_failed | 业务 transition 记录追加失败 | 最终化必须原子失败，不宣称完成 | [Q:101] [Q:97] |
| activation.manifest_mismatch | build/catalog/schema/template/source 绑定不同 | CoreUnready 或阻断 Unit，不晋级 | [Q:101] [Q:79] |
| activation.generation_conflict | generation CAS 失败 | Blocked，不切换完成所有者 | [Q:101] [Q:80] |
| activation.owner_conflict | 同一 occurrence 存在竞争物理完成所有者 | Blocked，不进行第二次物理发送 | [Q:101] [Q:13] |
| activation.core_unready | 共享 authority 或存储不可用 | CoreUnready，阻断生产就绪 | [Q:101] [Q:12] |
| activation.producer_unready | 注册生产者的依赖不可用 | 隔离该生产者，并使部署就绪失败 | [Q:101] [Q:12] |
| shadow.semantic_diff | 类型化 decision/hash/reason/proposal 不一致 | 阻断晋级并保留比较证据 | [Q:101] [Q:83] |
| shadow.side_effect_attempted | shadow 尝试 provider 重取、LLM、写入、发送或订单 | 拒绝动作并阻断晋级 | [Q:101] [Q:83] |
| operator.unauthorized | 认证身份不具备已批准权限 | 拒绝请求并审计拒绝 | [Q:101] [Q:47] |
| operator.evidence_invalid | 人工证据缺失、无效或披露过多 | 拒绝人工处置 | [Q:101] [Q:55] |
| operator.resolution_conflict | 人工 expected_version 或绑定冲突 | ResolutionRequired，不盲目覆盖 | [Q:101] [Q:87] |

## 适配器一致性合同（PROPOSED）

本表是 P01/N02 与通用 coordinator 的完整最低一致性规则，不能用孤立 Q27
引用替代。P01 自动、补偿与启动恢复共享业务 occurrence/完成所有者；N02 保留
窗口、reservation、attempt 结算的专用 authority，不能复制第三套回执或完成真相。
shadow_side_effects=None 排除再次 provider/LLM/业务查询、DB 写入、游标推进、
订单和发送；只有 active 路径可持久化 intent 并调用 authority。[Q:16] [Q:27]
[Q:33] [Q:34] [Q:78] [Q:83] [unit:MU-p01] [unit:MU-news-flash-aggregate]

| 规则 | 适用对象 | 规范值 | 依据 |
| --- | --- | --- | --- |
| p01_owner_group | MU-p01 | SharedBusinessOccurrenceOwner | [Q:16] [producer:p01-scheduled] [producer:p01-compensation] [producer:startup-resume-preopen-news-hot] |
| n02_authority | MU-news-flash-aggregate | PreserveWindowReservationAttemptSettlement | [Q:27] [producer:news-flash-aggregate] |
| application_contract | GenericCounted,P01Dedicated,N02Dedicated | OneApplicationResultAndFinalizerContract | [Q:27] [Q:78] |
| facts_instance | Active,Shadow | SameImmutablePreparedFactsIncludingModelOutputs | [Q:33] [Q:72] |
| projection | project | Pure | [Q:34] |
| shadow_compare | Shadow | JobDecision,SemanticProjection.sha256,rendered_sha256,ReasonCode,completion_proposal | [Q:19] [Q:83] |
| shadow_exclusions | Shadow | attempt_id,latency,diagnostic_timestamp | [Q:40] |
| shadow_side_effects | Shadow | None | [Q:33] [Q:34] [Q:83] |
| payload_drift | SameIntent | ResolutionRequired | [Q:89] |
| empty_source | Prepare | VerifiedEmptyOnly | [Q:85] |
| weak_authority | COMPAT,LocalAudit,SinkAttempt,Ok,Log | NeverTransportAccepted | [Q:7] [Q:78] |

上述合同是 PROPOSED，不宣称当前 Rust 已存在这些 RFC 类型或通用 finalizer。
来源失败不得重标为 NoData；人工接受、AlreadyTerminal、重试与补偿都必须遵守
终态完成合同，不能降低绑定要求。[Q:7] [Q:27] [Q:46] [Q:78] [Q:85] [Q:87] [Q:89]

## Task2 验证边界

`check-rfc.rb --root ROOT --draft` 与 `--check` 执行相同内容、哈希、引用检查。
strict 当前另行拒绝 PROVISIONAL；完整发布/HTML/CI 门禁由 Task5/6 补齐。
RFC 校验不代表部署、运行时迁移、回执或 WBS 工期已验收。
本校验器检查冻结 catalog/evidence 字节及引用成员关系；真实 Rust 符号字节新鲜度
由 `check-catalog.rb --root ROOT --draft` 与来源门禁独立验证，两者不能相互替代，
也不重新查询生产状态。[Q:5] [Q:42] [Q:69] [Q:92] [Q:105]
