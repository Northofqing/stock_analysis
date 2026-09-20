# 推送系统实施 RFC

状态：**PROVISIONAL**。版本：`push-system-rfc-v1`。本文定义 PROPOSED 应用合同，
不代表运行时代码已经实现、部署或取得远端回执。Task2 仅交付领域类型、应用结果、
源码映射和 ReasonCode；Task3 增补 DDL/恢复设计，Task4 增补运行与验收合同，WBS 由 Task5 承接，
本文不宣称这些合同已实现、部署或通过独立复审。[Q:56] [Q:61] [Q:69] [Q:108]

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

只有共享原子 completion owner 的 producer/occurrence 家族才可分组；以冻结 52 Unit 目录
为准，不因同 kind、文件或阶段合并。强路径只做 conformance 校正，STARVED/OPT-IN 保持原状。
C0–C6 是能力标签，不是遗漏非活跃目录项或重排物理 owner 的理由。[Q:1] [Q:16] [Q:21] [Q:32]

实施顺序为 Foundation → 原子 Unit 切片 → 尾部清理。四时段仅是 Epic，
不是物理完成所有者的切换边界。本文不证明 Unit 晋级、HTML/CI 发布、运行时部署
或用户已收到消息。[Q:16] [Q:17] [Q:23] [Q:26] [Q:42] [Q:74]

## 阅读导航与规范化规则（PROPOSED）

后续章节依次定义字段、JobDecision、DeliveryResult、完成分支、身份与终态合同、
CURRENT 映射和状态、ReasonCode、适配器一致性。引用使用方括号内的
`Q:编号`、`unit:ID`、`producer:ID`、`evidence:ID`，以及规范表声明的
`acceptance:ID`、`gate:ID`、`milestone:ID`、`publication:ID`；校验器实际解析并核对
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
| schedule.not_trading_day | 交易日 authority 确认为非交易日 | 仅调度评估/观测原因；不创建交易时段 occurrence 或 NoData/Disabled intent | [Q:101] [Q:28] |
| schedule.window_not_open | 捕获业务时间早于有效窗口 | 保持 occurrence 待处理，直到具备资格 | [Q:101] [Q:28] |
| schedule.window_expired | 捕获业务时间超过补偿窗口 | 仅产生策略允许的 schedule 提案，不暗示投递 | [Q:101] [Q:28] |
| schedule.occurrence_closed | 精确调度 occurrence 已关闭 | 不产生新调度，通知状态仍独立 | [Q:101] [Q:86] |
| schedule.occurrence_conflict | occurrence 的原状态、版本或转换提交前提冲突 | 零写作用拒绝，重读原 occurrence 与当前 fence 后重新评估；禁止盲重试 | [Q:77] [Q:86] [Q:101] |
| schedule.window_open | authority 授予有效窗口资格 | 按 current version/gate 进入 Eligible | [Q:28] [Q:101] |
| schedule.deferred | 策略允许下一 eligible session | 保存原 identity 和下一资格引用 | [Q:86] [Q:101] |
| input.source_recovered | 来源证据及版本已恢复 | 记录恢复事件，重验窗口与 readiness | [Q:30] [Q:101] |
| activation.ready | 共享和作用域合同均校验通过 | 生成 Ready snapshot | [Q:12] [Q:101] |
| input.source_unavailable | provider 读取失败 | BlockedOnInput，仅允许有界发送前重试 | [Q:101] [Q:12] |
| input.source_unready | 已注册合同下某 occurrence 的来源证据不可用 | BlockedOnInput；ACTIVE producer 合同缺失须用 activation.producer_unready 升级 ProducerUnready | [Q:101] [Q:12] |
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
| operator.not_delivered | 经认证处置并重查精确不投递终态 | 仅按不投递终态合同 CAS 到 NotDelivered；无通知游标或重发 | [Q:39] [Q:45] [Q:87] |
| operator.unauthorized | 认证身份不具备已批准权限 | 拒绝请求并审计拒绝 | [Q:101] [Q:47] |
| operator.evidence_invalid | 人工证据缺失、无效或披露过多 | 拒绝人工处置 | [Q:101] [Q:55] |
| operator.resolution_conflict | 人工 expected_version 或绑定冲突 | ResolutionRequired，不盲目覆盖 | [Q:101] [Q:87] |
| intent.created | 冻结事实形成新稳定 intent | 只提交业务 outbox，不意味着已发送 | [Q:76] [Q:101] |
| intent.no_data | 经过验证的来源明确为空且策略允许 | 保留空证据，仅独立 schedule 提案 | [Q:85] [Q:86] [Q:101] |
| intent.dispatch_claimed | lease 与版本 CAS 成功 | 进入或维持等待 authority，不解释为接受 | [Q:90] [Q:101] |
| intent.authority_verified | 私有 authority 重查精确绑定且策略允许 | 仅形成最终化资格，不提前宣布业务完成 | [Q:78] [Q:101] |
| finalizer.completed | 单一业务事务完成事实与事件共同提交 | 提交后确认，丢失确认只幂等重查 | [Q:97] [Q:101] |
| activation.applied | 已认证操作员执行批准代且 journal 提交 | 记录已执行事实，仍须与期望和实际 owner 审计一致 | [Q:98] [Q:99] [Q:101] |

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

## 业务持久化范围与 SQL 字节合同（PROPOSED）

Task3 在本文增加可执行的 SQLite 设计与恢复规范，仍不代表运行时实现、生产迁移、
部署或发送已验证。唯一 DDL 来源为同目录 `push-system-foundation.v1.sql`；
下方嵌入由原始文件逐字节复制，SHA 位于区域外。校验器只读核对文件、唯一 marker、
唯一 SQL fence 与 SHA，不执行不受信任 SQL；可执行性和约束由公开 SQLite 临时库
测试独立验证，不能用哈希一致替代行为测试。[Q:69] [Q:76] [Q:97]

所有业务连接必须在事务外启用 `foreign_keys=ON`、`recursive_triggers=ON`。
`push_foundation_schema.version=1` 是可查询版本；首次全新纳管域才建立并登记对象，
再次执行必须先核对入口快照与冻结登记，不得静默补建缺失对象。不清表、不覆盖、
不修改业务行，也不修复不兼容 schema；未来变更另行批准迁移。[Q:79] [Q:82]

`push_intents` 保存业务决定，只有 Ready 来源行承担发送 outbox；非发送行不伪造
PreparedPush 或第三套回执。先按
`IdentityRule::PreparedPushIntent` 规范化生成稳定身份，使用普通 INSERT；冲突时
回滚并只读比较身份及全部不可变材料，完全一致只幂等观察。相同身份的 payload、
rendered、evidence、template 或 source-contract 哈希漂移不得 UPDATE/REPLACE；
保留原材料与冲突证据，重读版本后 CAS 到 `ResolutionRequired` 并阻断该 Unit
晋级。不得把任一材料哈希加回身份，也不得换 decision ID 逃逸。版本冲突同样先
回滚、重读、隔离；若数据库不可写，外层门禁持续阻断直到隔离持久化成功。
已完成行后来冲突也可隔离，但不得撤回既有游标或重发。[Q:77] [Q:89] [Q:97]

初始版本为零，immutable job_decision_kind 分别将 Ready/NoData/Disabled 配对到
PendingDispatch/NoData/Disabled；只有 Ready 的创建事实同时是发送 outbox，
不伪造一次发送事件。`NoData` 仅来自经过验证的空事实，`Disabled` 仅来自明确
禁用；二者按 Task2 CompletionPolicy 独立提出 schedule 关闭，均不推进通知游标。
`ResolutionRequired` 禁止自动解封，只允许记录 lease/处置观察的同态版本推进，
或在 job_decision_kind=Ready、已认证人工处置清除冲突、原身份精确终态重验
和策略允许后以新版本 CAS
恢复到 `AwaitingFinalizer`；后者仍须步骤五重验并走步骤六，不授权再次发送。
同一 decision 的 Uncertain 隔离还可经不投递终态合同进入 `NotDelivered`，且该终态无离开边；
不能用已接受历史或任意材料冲突代替 Uncertain 来源。表内 `SameState` 是保持当前状态的规则
标识，不是数据库枚举。Received/Accepted/Rejected/Uncertain 均不是业务状态。
[Q:28] [Q:75] [Q:85] [Q:86]

每次状态/lease 更新必须带 `WHERE intent_id=? AND version=? AND
lease_generation=?`，派发/最终化还要匹配 owner、有效 until 与预期状态；
设置 `previous_state=state, version=version+1` 与本边规范 ReasonCode，并在同库追加
reason 完全一致的事件；同命名空间的任意代码不等于本边合法代码。首次 claim
或过期接管增加 generation；其他 owner 的未过期 lease 不可抢占。时间由可信
捕获业务时钟提供，SQL 不把调用方随意填写的时间当身份认证。释放只允许当前
owner 带 generation/version CAS；过期接管仍须查询原 durable attempt，不能把
lease 到期解释为发送许可。[Q:76] [Q:90]

`push_intent_transitions` 仅存引用身份和 binding hash，不存回执正文。
`event_id` 是规范化域 `IntentTransitionV1` 下
`(intent_id,expected_version,result_version)` 的稳定 SHA-256；第一事件的
`previous_sha256=NULL`，后续必须等于同 intent 前一版本事件的 canonical hash。
canonical hash 使用 Task2 规范化，包含事件除自身 hash 外的所有持久字段；
应用必须重算核验，SQLite 约束格式、版本链、当前 intent 的前态/结果态及 reason 匹配，
不宣称能在标准 SQLite 中验证 SHA 运算或 authority。只有进入 Completed/NotDelivered 的事件
可且必须带 terminal_ref_id、terminal_disposition 和 terminal_binding_sha256；前者处置只能是
Accepted/ManualConfirmedAccepted。后者另必须带与 intent 原 durable_decision_id 相等的
terminal_decision_id、operator_audit_ref 与 operator_audit_sha256；其他事件这组字段全空。
这只是引用和哈希，不是复制回执，更不是凭字段非空完成认证。
[Q:78] [Q:87] [Q:97]

所有 `*_sha256` 是 64 位小写十六进制，`build_commit` 是 40 位；每列同时检查
TEXT 存储类型、字符长度、CAST AS BLOB 字节长度和 lower-hex，拒绝 NUL 隐藏后缀。
intent_id 与两类 event_id 同样是 64 位 SHA 文本；durable_decision_id/terminal_ref_id
仍为 opaque 标识，不误收紧。ReasonCode 必须是无 NUL 的 ASCII TEXT。
这些格式检查不是内容真实性证明。时间是非负 i64 UTC 微秒，业务日期必须是真实 YYYY-MM-DD；
lease owner/until 成对可空，首代前驱与首事件前驱可空，其余 NULL 条件由 DDL
约束。`ReasonCode` 只允许 Task2 的九个命名空间，规范表逐项指定语义；
扩展成功动作代码见已有注册表，诊断文案不能驱动转换。[Q:79] [Q:97] [Q:101]

`push_activation_manifests` 是不可变期望状态，每代绑定 build/Git、catalog、两个
schema、template、source-contract、证据、批准身份/时间、窗口和物理 owner；
manifest 的 canonical SHA 由应用重算验证。下一代以旧 generation 和前驱身份
为 CAS 条件 INSERT，唯一约束处理竞争。`push_promotion_journal` 是独立已执行
事实，稳定事件身份使用 `PromotionV1(unit_id,generation)` 的 SHA-256，前驱和
canonical 规则同上；六种成功 journal action 的 reason 必须精确为 activation.applied；namespace 相同仍不代表
成功边合法。manifest FK 绑定全部版本哈希，不复制另一套版本真相。
批准/执行者须先经外部认证和授权，SQL 非空 actor 或与批准者相等不构成认证。
[Q:79] [Q:80] [Q:81] [Q:98] [Q:99]

正常操作顺序为批准新 manifest、本地准备、排空/切换内存 owner、追加执行 journal；
外部/内存 owner 切换与 SQLite 并不原子。任意中断或提交确认丢失必须先阻断就绪，
重查 journal、manifest 与实际 owner，审计一致才确认执行。不得在缺 journal 时
把期望状态当已执行，也不按日志自动激活。一个未执行代必须先协调，禁止跳代。
回滚写新 generation 指向同 Unit 的兼容历史目标并恢复 owner；只允许逻辑回滚
或 Foundation 兼容 N-1，禁止删除未决数据、改历史或破坏性降 schema。[Q:80]
[Q:82] [Q:98] [Q:99]

CURRENT 对照仅证明已有专用 authority，绝不证明上述业务表已接线：
`schedule_occurrence_identity`（src/bin/monitor/p01.rs:322）、
`run_p01_compensation_once`（同文件:1462）共享 P01 occurrence；
`begin_attempt`（src/durable_delivery/coordinator.rs:4793）、
`recover_one_expired_attempt`（同文件:5233）、
`reacquire_rejected`（同文件:4736）是冻结恢复证据。
通用、P01、N02 必须沿 Task2 适配器合同重查各自 authority；本业务 schema 不以
PushKind 或 count 值替代专用 owner。[Q:16] [Q:27] [unit:MU-p01]
[unit:MU-news-flash-aggregate] [producer:p01-scheduled] [producer:p01-compensation]
[producer:news-flash-aggregate] [evidence:p01-identity] [evidence:p01-compensate]
[evidence:startup-begin-attempt] [evidence:startup-expired-attempt] [evidence:startup-reacquire]

## 业务 outbox 字节恢复合同（PROPOSED）

步骤一的 Ready 行必须在同一行提交首次 PreparedPush 规范化快照与首次 render 原始字节，
不能只保存不可逆的 hash 后在重启重新构造。prepared_push_bytes 保存按 Task2
规则序列化的 PreparedPush（其中外部原始字节以 SHA/长度编码），rendered_bytes
独立保存其首次原始输出；payload_sha256 对前者求 SHA，rendered_sha256 对后者
求 SHA。重启先解析快照、重算并比对全部关联材料/身份/绑定与原始字节长度，
任一不一致隔离到 ResolutionRequired，不再次 provider/LLM/render。
初始 NoData/Disabled 没有 PreparedPush，prepared_push_bytes/rendered_bytes 与
payload_sha256/rendered_sha256 必须整组 NULL，禁止 dummy 内容或部分 NULL。
job_decision_kind 不可变，所以这些非发送事实隔离后仍保持 NULL，不能转发送最终化。
Ready 来源后来按策略转 NoData/Disabled 时保留原整组材料，不能抹除或伪造来源。
SQLite 对 Ready 组约束 BLOB/非空，对非发送组约束整组 NULL，并对两组都约束
immutable；不能原生证明 SHA 与内容一致。这些应用
重算与恢复器仍是 PROPOSED。保存的是发送输入，不是回执或新 authority。[Q:33]
[Q:72] [Q:76] [Q:89] [unit:MU-p01] [producer:p01-scheduled] [evidence:p01-once]

| 规则 | 材料 | 规范值 | 依据 |
| --- | --- | --- | --- |
| prepared_snapshot | prepared_push_bytes | 首次 PreparedPush 规范化字节不可变保存 | [Q:33] [Q:76] [Q:89] |
| first_render | rendered_bytes | 首次 render 原始字节不可变保存 | [Q:72] [Q:76] [Q:89] |
| content_binding | payload_sha256,rendered_sha256 | 应用重算 SHA 与长度并核对快照绑定；SQLite 仅检查格式 | [Q:76] [Q:89] |
| restart_reuse | prepared_push_bytes,rendered_bytes | 只读取原字节；禁止重新 provider/LLM/render | [Q:33] [Q:72] [Q:76] |
| drift | SameIntent | 保留原字节并隔离 ResolutionRequired；禁止 UPDATE/REPLACE 覆盖 | [Q:77] [Q:89] |

## 持久化条件组与兼容守卫（PROPOSED）

本表固定本轮 v1 的可解析最低约束，不能以近似文字替代。[Q:75] [Q:76] [Q:79]
[Q:82] [Q:89] [Q:97] [Q:101]

| 规则 | 对象 | 规范值 | 依据 |
| --- | --- | --- | --- |
| decision_origin | job_decision_kind | Ready/NoData/Disabled 是不可变的初始业务决定 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| ready_group | prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256 | Ready 必须整组非空且不可变 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| non_send_group | prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256 | 初始 NoData/Disabled 必须整组 NULL；隔离后仍保持 NULL | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| ready_non_send_state | Ready→NoData/Disabled | 保留原 Ready 字节与哈希，不改变 job_decision_kind | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| edge_reason | push_intents.reason | 仅允许业务转换表的逐边 ReasonCode，不接受同命名空间任意代码 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| event_reason | push_intent_transitions.reason | 必须等于本次 CAS 后的 intent.reason | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| canonical_identifiers | intent_id,transition.event_id,promotion.event_id | 64 位小写十六进制 TEXT；应用重算身份内容 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| hash_storage | Sha256/GitSha40 | 同时校验 TEXT 类型、字符长度、BLOB 字节长度与小写十六进制 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| compat_entry | SQLite CLI schema script | .bail on；持久化 DDL 前快照对象，不用事后补建掩盖缺失 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| compat_inventory | 25 个明确 name/type | metadata 与保护 trigger 纳管；拒绝挂在纳管表上的额外 trigger/index；保留独立无关表 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |
| compat_trust | v1 固定兼容签名与冻结定义登记 | 比较已登记入口对象字节，不认证同时伪造 metadata 与保护对象的恶意管理员 | [Q:76] [Q:79] [Q:89] [Q:97] [Q:101] |

`push-system-foundation.v1.sql` 是 SQLite CLI schema script，首行 `.bail on`
保证计划接口 `/usr/bin/sqlite3 TEMP_DB < file` 遇到任意错误立即退出，不继续后续
DDL；它不是可直接交给 library execute_batch 的纯 SQL。运行时接线仍未实现。
临时测试也使用不附加 `-bail` 的原始 CLI 标准输入，单独证明这一行为。[Q:76] [Q:82]

脚本在事务开始、任何持久化 CREATE 之前，按 TEMP allowlist 冻结入口的对象集合、
类型和 sqlite_master.sql 原始字节。25 个明确名称覆盖六表、一个显式索引、十八个
trigger；metadata 两表及其保护 trigger 同样纳管。另拒绝挂在这些表上的清单外
显式 trigger/index；SQLite 自动索引由表定义约束，不作为独立 SQL 对象登记。
独立的无关表（包括 push_legacy）不影响 fresh 判定，不被登记或改动。[Q:79] [Q:82]

只有完全没有纳管对象的库才首次创建并登记 `push_foundation_objects`。既有库
必须已有唯一 v1 版本行、正确固定 schema_signature、完整的冻结定义登记，且入口
对象集合/类型/定义与登记逐字节相等；缺对象、弱同名 trigger、额外挂表对象、
版本/签名不符均先失败，不能补建后自称兼容。metadata 不允许后补登记/改写；
同一正确 v1 才可重执行并保留全部行与原字节。[Q:79] [Q:82] [Q:98]

固定签名为 `dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd`，是
`push-foundation-v1-final-wave1-not-delivered` 的版本身份，不是 SQLite 对 DDL 计算的内容摘要。本修订增加 NotDelivered 及事件证据组、
收紧六个 activation 成功边；旧 task3-r1 v1 签名不兼容，入口即拒绝，不能自动升级。
守卫证明受信首次登记后的定义未漂移；不验证任意存量业务行语义，不认证拥有
任意 schema 写权限、同时伪造 metadata 与保护对象的恶意管理员，也不检查无关
独立表的内部定义。首次部署只执行已通过文件字节和行为门禁的脚本；本任务不
迁移既有不兼容库，失败后保留原库供另行批准处理。[Q:79] [Q:82] [Q:98]

## 业务意图转换（PROPOSED）

本表中的首次派发与恢复，均须遵守 outbox 条件组及不可变字节合同。[Q:76] [Q:89]

本表为可解析的 v1 规范；标识和值均参与精确校验。[Q:75] [Q:76] [Q:77] [Q:89] [Q:97]

| 起点 | 终点 | 发起者 | 前置条件 | 持久副作用 | 禁止副作用 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| None | PendingDispatch | 应用 | 已冻结事实与稳定身份 | 插入版本零 intent/outbox | 派发先于提交 | intent.created | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| None | NoData | 应用 | 已验证为空且策略允许 | 插入版本零并保留空证据 | 伪造终态引用或推进通知游标 | intent.no_data | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| None | Disabled | 应用 | 显式禁用且策略允许 | 插入版本零禁用事实 | 把未就绪当禁用或推进通知游标 | policy.disabled | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| PendingDispatch | AwaitingAuthority | dispatcher | 有效 lease 与 expected-version CAS | 同库 CAS 并追加事件 | 先发后存或新建逃逸身份 | intent.dispatch_claimed | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| PendingDispatch | NoData | 应用 | 冻结空证据与策略及版本 CAS | 同库 CAS 并追加事件；保留原 Ready 材料 | 把来源错误当空或推进通知游标 | intent.no_data | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| PendingDispatch | Disabled | 应用 | 显式禁用及版本 CAS | 同库 CAS 并追加事件；保留原 Ready 材料 | 清除待处理事实或推进通知游标 | policy.disabled | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| AwaitingAuthority | AwaitingFinalizer | authority 适配器 | 私有重查精确绑定且策略允许 | 同库 CAS 并追加事件 | 仅凭日志或结果枚举晋级 | intent.authority_verified | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| AwaitingFinalizer | Completed | finalizer | 再次精确绑定且策略允许及版本 CAS | 同一事务执行完成事实 CAS 与事件 | 跨库原子性或跳过事件 | finalizer.completed | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| AwaitingAuthority/ResolutionRequired | NotDelivered | 已认证操作员与私有 authority 适配器 | 不投递终态合同的来源、精确绑定、独立审计及版本 CAS 全通过 | 同库 CAS 与不可变处置事件；解除未决阻断但保留失败 | 推进游标、重发、撤销 Accepted 或计入成功 | operator.not_delivered | [Q:39] [Q:45] [Q:78] [Q:97] |
| PendingDispatch/AwaitingAuthority/AwaitingFinalizer/Completed/NoData/Disabled | ResolutionRequired | 应用或 finalizer | 材料或版本冲突并以重读版本 CAS | 保留原材料与终态历史并阻断 Unit 晋级 | 覆盖材料或撤销既有游标 | intent.payload_conflict/intent.expected_version_conflict/finalizer.cas_conflict | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |
| AwaitingAuthority/AwaitingFinalizer | ResolutionRequired | 私有 authority 适配器 | 未知或处置冲突经重查且版本 CAS | 隔离并保留原 decision 与处置证据 | 自动重发或自动推进通知游标 | transport.uncertain/operator.resolution_conflict | [Q:78] [Q:87] [Q:97] |
| ResolutionRequired | AwaitingFinalizer | 已认证操作员与私有 authority 适配器 | Ready 来源且处置清除冲突与原身份精确接受绑定、策略及版本 CAS | 保留处置证据并只恢复最终化资格 | 自动解封或再次发送 | intent.authority_verified | [Q:77] [Q:78] [Q:87] [Q:97] |
| PendingDispatch/AwaitingAuthority/AwaitingFinalizer/ResolutionRequired | SameState | lease 管理者 | owner/until/generation 与版本 CAS | 版本加一并追加事件 | 抢占未过期外来 lease | intent.lease_held/intent.dispatch_claimed | [Q:75] [Q:76] [Q:77] [Q:89] [Q:97] |

| AwaitingAuthority | SameState | authority 恢复器 | 原 decision 与版本 CAS | 仅记录拒绝或审计阻塞并追加事件 | 盲重发或提前最终化 | transport.rejected/finalizer.terminal_ref_invalid | [Q:78] [Q:85] [Q:97] |
| AwaitingFinalizer | SameState | finalizer | 原绑定重查失败与版本 CAS | 只保留阻塞原因并追加事件 | 推进完成或绕过重验 | finalizer.terminal_ref_invalid | [Q:78] [Q:85] [Q:97] |

## 激活转换（PROPOSED）

本表为可解析的 v1 规范；标识和值均参与精确校验。[Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99]

| 起点 | 终点 | 发起者 | 前置条件 | 持久副作用 | 禁止副作用 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| None | Disabled | 已认证操作员 | 批准身份与初始 generation=1 | 新 manifest 与 Initialize journal | 仅凭 manifest 宣称执行 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |
| Disabled | Shadow | 已认证操作员 | 全部版本绑定与 generation CAS | 新 manifest 与 EnterShadow journal | shadow 外部副作用 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |
| Shadow | Active | 已认证操作员 | 证据通过且无 ResolutionRequired 与 generation CAS | 新 manifest 与 Activate journal | 双物理 owner 或自动批准 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |
| Active | Draining | 已认证操作员 | generation CAS 与停止新增派发 | 新 manifest 与 Drain journal | 删除未决事实或中断恢复 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |
| Draining | Disabled | 已认证操作员 | 排空证据与 generation CAS | 新 manifest 与 Disable journal | 把未决状态当已完成 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |
| Disabled/Shadow/Active/Draining | RollbackTarget | 已认证操作员 | 新 generation CAS 与同 Unit 兼容历史目标 | 新 manifest 与 Rollback journal 恢复目标 owner | 改写历史或破坏性 schema 回滚 | activation.applied | [Q:79] [Q:80] [Q:81] [Q:82] [Q:98] [Q:99] |

## 权威处置与最终化资格（PROPOSED）

本表为可解析的 v1 规范；标识和值均参与精确校验。[Q:78] [Q:86] [Q:87]

| 起点 | 终点 | 发起者 | 前置条件 | 持久副作用 | 禁止副作用 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Accepted | AwaitingFinalizer | 私有 authority 适配器 | 终态已封存且 TerminalBinding 与 CompletionPolicy 均通过 | 仅记录完成提案后进入步骤六 | 把远端接受当业务完成 | intent.authority_verified | [Q:78] [Q:86] [Q:87] |
| ManualConfirmedAccepted | AwaitingFinalizer | 私有 authority 适配器 | 精确绑定且 AcceptedOrManualBound 策略允许 | 独立人工接受指标与完成提案 | 伪装 TransportAccepted | intent.authority_verified | [Q:78] [Q:86] [Q:87] |
| AlreadyTerminal | DispositionDependent | 私有 authority 适配器 | 重查 TerminalBinding 并逐处置执行终态完成合同 | 接受仅推进完成；不投递仅按专门合同收敛 | 全处置推进或省略绑定 | intent.authority_verified/operator.not_delivered/transport.rejected/transport.uncertain | [Q:78] [Q:86] [Q:87] |
| AcceptedAuditPending/AcceptedTaskTransitionPending | AwaitingAuthority | 恢复器 | authority 尚未封存 | 仅恢复审计与 authority 内部转换 | 重发或业务最终化 | finalizer.terminal_ref_invalid | [Q:78] [Q:86] [Q:87] |
| Rejected | AwaitingAuthority | dispatcher | 当前显式重试授权及原 decision 与 lease CAS | 仅授权时申请新 attempt | 盲重试或推进游标 | transport.rejected | [Q:78] [Q:86] [Q:87] |
| Uncertain | ResolutionRequired | 恢复器 | 权威不确定性已确认 | 隔离并等待已认证人工解析 | 自动重发或自动清理 | transport.uncertain | [Q:78] [Q:86] [Q:87] |
| ManualConfirmedNotDelivered | NotDelivered | 已认证操作员与私有 authority 适配器 | 不投递终态合同的来源、精确绑定、独立审计及版本 CAS 全通过 | 同库追加不投递终态事实；保留失败指标 | 推进游标、重发或冒充接受 | operator.not_delivered | [Q:78] [Q:86] [Q:87] |
| COMPAT/Blocked | AwaitingAuthority | 应用 | 无强 authority 终态 | 仅保留弱证据或阻塞诊断 | 构造 VerifiedTerminalRef 或权威完成 | finalizer.terminal_ref_invalid | [Q:78] [Q:86] [Q:87] |

## 跨库恢复顺序（PROPOSED）

本表为可解析的 v1 规范；标识和值均参与精确校验。[Q:76] [Q:78] [Q:90] [Q:97]

| 步骤 | 执行者 | 幂等键 | 已提交可见事实 | 重启扫描 | 下一合法动作 | 禁止行为 | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 业务应用 | intent_id | 业务 intent/outbox 本地提交 | 按稳定身份查询是否已存在 | 比较不可变材料并取得 lease | 派发先于提交或跨库事务 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 2 | dispatcher | durable_decision_id | durable reserve/claim | 按原 decision 查询 reservation 与 lease | 确认未尝试且满足 fencing 后进入步骤三 | 外来 lease 抢占或新建身份 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 3 | authority | durable_decision_id+attempt_id | durable attempt 先于外部尝试记录 | 查询 attempt 与不确定状态 | 仅已有合法 attempt 执行一次或进入恢复 | 在途未知结果盲重发 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 4 | authority | durable_decision_id+attempt_id | durable terminal 本地提交并封存 | 查询原 terminal 及未封存审计 | 封存后进入步骤五 | 以 sink attempt 或审计日志冒充终态 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 5 | 私有 authority 适配器 | IdentityRule::TerminalBinding | 只读重验不产生新投递事实 | 从原 authority 再查引用与绑定 | 资格允许才进入步骤六 | 复制回执或跨事务复用未重验引用 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 6 | finalizer | intent_id+expected_version+event_id | 一个业务事务的 CAS 与 transition 共同提交 | 查业务状态版本和稳定事件 | 失败整体回滚并重查；成功进入步骤七 | CAS 零行追加或事件失败仍提交 | [Q:76] [Q:78] [Q:90] [Q:97] |
| 7 | 业务应用 | intent_id+result_version | 提交后的确认与独立完成指标 | 查询既有 Completed/NotDelivered 和事件 | 幂等返回原终态事实与独立指标 | 丢失确认导致二次发送或完成 | [Q:76] [Q:78] [Q:90] [Q:97] |

## 故障与提交确认矩阵（PROPOSED）

本表为可解析的 v1 规范；标识和值均参与精确校验。[Q:76] [Q:82] [Q:88] [Q:90] [Q:100]

| 故障标识 | 已提交事实 | 恢复扫描 | 重发许可 | 幂等键 | 目标状态 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| before_intent_commit | 无新业务事实 | 按 intent_id 重算后查库 | 仅首次且完整门禁通过 | intent_id | PendingDispatch | intent.created | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_intent_commit | intent/outbox | 扫描未完成 intent | 查询 durable 后仅允许首次 | intent_id+durable_decision_id | PendingDispatch | intent.created | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| before_claim | intent/outbox | 按原 decision 查询 reservation | 仅确认无 attempt 后首次 | durable_decision_id | AwaitingAuthority | intent.dispatch_claimed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_claim | reservation | 查询 claim 与有效 lease | 仅确认未尝试且 lease 有效 | durable_decision_id | AwaitingAuthority | intent.dispatch_claimed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| before_attempt | reservation | 查询是否已记录 attempt | 仅确认未尝试且 lease 有效 | durable_decision_id+attempt_id | AwaitingAuthority | intent.dispatch_claimed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_attempt | attempt 可能已外发 | 查询原 attempt 并协调未知结果 | 否 | durable_decision_id+attempt_id | ResolutionRequired | transport.uncertain | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| before_terminal_commit | attempt 或待封存审计 | 恢复原 authority 并查询未知结果 | 否 | durable_decision_id+attempt_id | AwaitingAuthority/ResolutionRequired | transport.uncertain | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_terminal_commit | durable Accepted terminal；确认可能丢失 | 查询原 terminal 并重新验证绑定 | 否 | IdentityRule::TerminalBinding | AwaitingFinalizer | intent.authority_verified | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| before_reverify | durable terminal | 私有 authority 重查绑定与资格 | 否 | IdentityRule::TerminalBinding | AwaitingFinalizer | intent.authority_verified | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_reverify | durable Accepted terminal；引用仅在内存 | 重新查询而非恢复内存引用 | 否 | IdentityRule::TerminalBinding | AwaitingFinalizer | finalizer.terminal_ref_invalid | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_business_cas | 旧业务提交事实；CAS 尚未提交 | 事务恢复回滚后查状态版本 | 否 | intent_id+expected_version+event_id | AwaitingFinalizer | finalizer.transition_append_failed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_transition_append | 旧业务提交事实；事件尚未提交 | 事务恢复回滚后查状态与事件 | 否 | intent_id+expected_version+event_id | AwaitingFinalizer | finalizer.transition_append_failed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| after_business_commit | Completed 与事件；确认可能丢失 | 查既有终态及稳定事件并幂等确认 | 否 | intent_id+result_version+event_id | Completed | finalizer.completed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| sqlite_busy | 最后一次提交事实 | 有界退避后查两库原身份 | 不得仅因 busy 重发 | intent_id+durable_decision_id | SameState | intent.lease_held | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| foreign_lease | 其他 owner 的有效 lease | 等 lease 到期并重新读 generation | 否 | intent_id+lease_generation+version | SameState | intent.lease_held | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| expired_lease | 过期 lease 与原 decision | generation 与版本 CAS 后查询 durable | 仅查询证明确未尝试或有显式拒绝重试授权 | intent_id+lease_generation+version | SameState | intent.dispatch_claimed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| accepted_audit_pending | Accepted 的未封存审计 | 仅修复 authority 审计和内部转换 | 否 | durable_decision_id+attempt_id | AwaitingAuthority | finalizer.terminal_ref_invalid | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| rejected_retry | 已封存 Rejected | 重新核对当前显式授权与 lease | 仅显式授权产生新 attempt | durable_decision_id+new_attempt_id | AwaitingAuthority | transport.rejected | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| uncertain | 权威未知结果 | 隔离并等待已认证人工解析 | 否 | durable_decision_id+attempt_id | ResolutionRequired | transport.uncertain | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| not_delivered_before_business_commit | 不投递 terminal 与独立 operator audit 已封存 | 重查原 decision 精确绑定及版本；仅恢复业务终态事务 | 否 | intent_id+expected_version+event_id | NotDelivered | operator.not_delivered | [Q:39] [Q:45] [Q:78] [Q:97] |
| not_delivered_after_business_commit | NotDelivered 与不可变事件；确认可能丢失 | 查询原事件与关联 audit；返回已处置失败而非接受 | 否 | intent_id+result_version+event_id | NotDelivered | operator.not_delivered | [Q:39] [Q:45] [Q:78] [Q:97] |
| payload_drift | 原身份及不可变材料 | 重读并 CAS 隔离；保留冲突证据 | 否 | intent_id | ResolutionRequired | intent.payload_conflict | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| expected_version_conflict | 获胜者提交事实 | 回滚本事务并重读 CAS 隔离 | 否 | intent_id+expected_version | ResolutionRequired | intent.expected_version_conflict | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| terminal_ref_invalid | 原 durable 与业务事实 | 私有 authority 重新核验 | 否 | IdentityRule::TerminalBinding | AwaitingAuthority/AwaitingFinalizer | finalizer.terminal_ref_invalid | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| business_finalization_failure | 原 durable terminal 与旧业务事实 | 整体回滚后重查；只重做最终化 | 否 | intent_id+expected_version+event_id | AwaitingFinalizer | finalizer.transition_append_failed | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| activation_rollback | 历史 manifest 与已执行 journal | 核对最新已执行 generation 与兼容目标 | 回滚本身不授权发送 | unit_id+new_generation | RollbackTarget | activation.applied | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| promotion_commit_ack_lost | 新 generation journal 可能已提交 | 按 Unit/generation 查询而非再执行切换 | 否 | unit_id+generation+event_id | ExistingExecutedState | activation.generation_conflict | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |
| manifest_without_journal | 新期望 manifest；尚无执行事实 | 比较 manifest 与 journal 并阻断就绪 | 否 | unit_id+generation | Blocked | activation.manifest_mismatch | [Q:76] [Q:82] [Q:88] [Q:90] [Q:100] |

## 最终化事务与恢复边界（PROPOSED）

跨库顺序表的七步是有序本地提交，不存在业务 SQLite 与 durable SQLite 的共同
原子事务。步骤二至四由现有 authority 负责 reserve、fencing、attempt、审计和终态；
任何 audit pending 均不能提前进入业务最终化。步骤五每次从私有 authority 重查
精确引用及 CompletionPolicy，步骤六失败重试前再次重查，禁止复用未验证快照。
`AlreadyTerminal` 是处置容器而不是接受凭证；人工接受使用独立指标，不记成
TransportAccepted。ManualConfirmedNotDelivered 按不投递终态合同收敛到独立业务终态，
不是步骤六接受分支的完成资格。schedule 关闭、通知游标推进分别按策略写事实，不能互相代替。
[Q:76] [Q:78] [Q:86] [Q:87]

步骤六有互斥的接受完成与不投递终态分支；后一分支只按不投递终态合同落状态和事件，
绝不执行业务通知游标或其它接受完成副作用。两分支均必须使用一个业务连接的 `BEGIN IMMEDIATE` 事务包装：
先执行绑定 owner/lease/generation/expected-version 的 CAS，立即读取 affected rows；
零行执行 ROLLBACK、禁止追加事件，并重新查询原事件/状态区分提交确认丢失和真实
冲突。只有一行时才追加稳定 transition 与本库完成 owner 所需事实；所有步骤成功
才 COMMIT。若完成 owner 不在该业务库，不能把该 Unit 宣称已经原子迁移，必须在
后续原子 Unit 切片解决边界，Task3 不虚构跨库游标事务。[Q:16] [Q:76] [Q:97]

任意 SQL 错误（包括 CHECK、FK、UNIQUE、trigger、busy、COMMIT 错误）都必须终止
当前事务并 ROLLBACK，禁止捕获语句错误后仍提交 CAS。保护 trigger 使用
RAISE(ROLLBACK)，但 SQLite 通用 CHECK/UNIQUE 的默认 ABORT 只回滚该语句，
所以不能把 DDL 自身误宣称为完整事务包装器。临时测试采用独立 sqlite3
`-batch -bail` 连接：错误即退出并关闭连接，使整个未提交事务回滚；
分别验证 CAS 成功、零行零事件、事件绑定/格式/重复身份失败时旧状态版本不变。
运行时事务包装器尚未实现，不以此文档测试宣称部署。[Q:76] [Q:97] [Q:100]

步骤七确认丢失时重启只读取已存在的 Completed/NotDelivered 和事件并返回原终态结果，不再
执行完成副作用。步骤四确认丢失同理先查既有 durable terminal；无论丢哪一库的
确认，都不得通过新 decision/intent 身份再次发送。只有当前明确授权的 Rejected
才能申请新 attempt；在途未知、Uncertain、Accepted 审计未封存都不得盲重发。
NotDelivered 必须继续关联原证据且计入失败；它是已处置终态，但仍受全部清理资格与
严格保留期约束，绝不构成 Production Verified 成功样本。ResolutionRequired、非终态及相关证据不得自动清理，按最严格保留类别处理。
[Q:76] [Q:85] [Q:88] [Q:90] [Q:100]

本任务测试只连接新建临时 SQLite，不复制 data/** 或现存库，不调用 provider、
LLM、发送或订单。SQL 行为测试不是进程级 durable 故障注入，不证明生产两个库
的真实恢复；故障矩阵是后续 Unit 实现必须执行的合同。运行门禁、WBS、HTML/CI
发布仍留在 Task4/5/6，本 RFC 持续 PROVISIONAL。[Q:42] [Q:69] [Q:100] [Q:105]

## 类型：ScheduleOccurrence（PROPOSED）

创建者：PhaseScheduler，使用 catalog 绑定的交易日历/MarketSession authority。消费者：当前 owner 的 scheduler、恢复器及只读观测投影。[Q:16] [Q:28] [Q:86]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| schedule_occurrence_id | Sha256 | ScheduleIdentity::v1 | 派生且排除自身 |
| namespace | Namespace | Test 与 Production 隔离 | 纳入 |
| unit_id | UnitId | catalog 原子迁移身份 | 纳入 |
| producer_id | ProducerId | catalog producer | 纳入 |
| schedule_or_trigger_id | NonEmptyText | 注册的 schedule 或 event/manual trigger | 纳入 |
| calendar_id | NonEmptyText | catalog 绑定的交易日历版本引用 | 纳入 |
| business_date | Date | 仅 MarketSession authority 提供 | 纳入 |
| occurrence_family | NonEmptyText | catalog 声明的业务族 | 纳入 |
| occurrence_key | NonEmptyText | 族内稳定键；不取当前 tick | 纳入 |
| completion_owner | CatalogOwnerRef | catalog 原子完成 owner | 纳入 |
| source_contract_id | NonEmptyText | catalog 绑定来源合同 | 纳入 |
| window_start | UtcMicros | 含起点；由交易日历计算 | 纳入 |
| window_end | UtcMicros | 不含终点且大于起点 | 纳入 |
| catch_up_policy | CatchUpPolicy | 下表四种策略闭集 | 纳入 |
| status | ScheduleStatus | 生命周期表闭集 | 纳入 |
| version | u64 | ScheduleVersionRule::v1 | 纳入 |
| reason | ReasonCode | 当前转换的稳定原因 | 纳入 |
| created_at | UtcMicros | 首次创建时间 | 纳入 |
| updated_at | UtcMicros | 单调更新；不能代替 occurrence 自身的 version | 纳入 |

字段的规范化用于快照序列化；身份哈希仅取下一表的有序子集，不能误用整份快照。所有类型均是拟议合同，未新增运行时类型或表。

## 类型：ScheduleOccurrenceTransitionRequest（PROPOSED）

创建者：持有当前 gate/fence 的 scheduler、恢复器或 completion owner，读取持久 occurrence 后构造不可变请求。消费者：业务库的 occurrence 转换入口。[Q:28] [Q:77] [Q:80] [Q:86]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| schedule_occurrence_id | Sha256 | 必须引用已持久化的同一 occurrence | 纳入 |
| from_status | ScheduleStatus | 请求构造时读取的原状态，提交时必须仍相等 | 纳入 |
| to_status | ScheduleStatus | 仅允许调度生命周期表中的边 | 纳入 |
| expected_version | u64 | ScheduleVersionRule::v1 | 纳入 |
| expected_generation | u64 | 必须等于当前 Unit gate 的 generation | 纳入 |
| fence_token | ActivationFence | 完整绑定 unit_id、generation、manifest_sha256、physical_owner，不能使用缓存值授权 | 纳入 |
| reason | ReasonCode | 与对应生命周期边的 ReasonCode 相同 | 纳入 |
| evidence_refs | Vec<EvidenceRef> | 已获许可且不可变的窗口、来源或完成提案证据 | 纳入 |

`ActivationFence` 是前文 common_fence 的类型化四元组，generation 同时必须等于 expected_generation。请求不携带新的业务身份，也不以 push_intents.version 作为 expected_version。转换入口只校验已经准备好的不可变证据引用；自身不调用 provider/LLM 或发送，不以失败后的补采集掩盖 CAS 冲突。

## 调度版本与转换提交（PROPOSED）

[Q:28] [Q:77] [Q:80] [Q:86]。本表是 `ScheduleVersionRule::v1` 的唯一规范定义，覆盖尚无业务 intent 的 Expected、Eligible、Deferred、BlockedOnInput 以及其余全部调度状态。

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| storage_owner | ScheduleOccurrenceStateAndTransitionEvidence | BusinessDBSameTransactionIndependentOfPushIntentVersion | [Q:28] [Q:86] |
| initial_state | FirstUniqueOccurrenceInsert | Expected | [Q:28] |
| initial_version | FirstUniqueOccurrenceInsert | Zero | [Q:77] |
| create_conflict | ExistingScheduleOccurrenceId | ReadExistingNeverOverwriteOrReset | [Q:16] [Q:77] |
| request_guard | EveryLifecycleTransition | ExactIdFromStatusExpectedVersion | [Q:77] |
| fence_guard | EveryLifecycleTransition | CurrentUnitGenerationManifestOwnerAndExpectedGeneration | [Q:80] |
| lifecycle_guard | EveryLifecycleTransition | RegisteredEdgeReasonAuthorityWindowAndEvidence | [Q:28] [Q:86] |
| success_version | ExactlyOneRowCAS | CheckedExpectedVersionPlusOne | [Q:77] |
| overflow | ExpectedVersionAtU64Max | RefuseNoWritesNoEvents | [Q:77] |
| atomic_commit | SuccessfulTransition | StateVersionReasonAndTransitionEvidenceOneBusinessTransaction | [Q:86] |
| zero_rows | FailedCAS | NoStateOrVersionWriteNoEventNoPrepareProviderLLMSinkCursorOrder | [Q:77] |
| conflict_recovery | schedule.occurrence_conflict | RereadOccurrenceAndCurrentFenceReevaluateNeverBlindRetry | [Q:77] |
| identity_version | OccurrenceVersionAndRequestExpectedVersion | ExcludedFromScheduleOccurrenceId | [Q:16] |
| commit_ack_unknown | SameOccurrenceAndProposedResultVersion | RequeryOccurrenceAndVersionEventBeforeAnyNewRequest | [Q:77] |

首次按唯一 schedule_occurrence_id 插入 Expected、version=0 及创建证据，三者同一业务库事务提交；并发 create 只读取获胜的原 occurrence，不能重置版本。之后每次合法转换都要求持久行的 ID、from_status、version=expected_version 同时匹配，并在提交临界区重新校验完整当前 fence、expected_generation、逐边 authority/窗口/证据与 ReasonCode。fence 检查必须与 owner 晋级串行，不能检查后再用旧缓存提交。成功恰好更新一行，version 经溢出检查后严格加一；status、version、reason、updated_at 与 transition evidence 共同提交，任何写入或证据追加失败均整体回滚。

转换证据以 `(schedule_occurrence_id, result_version)` 唯一关联，保存 from_status、to_status、expected_version、result_version、当前 fence 引用、ReasonCode、输入证据 hash 和提交时间；创建证据使用 result_version=0。提交确认丢失时先重查该事件和行版本，已提交则返回既有事实；不得重复追加事件或把已成功转换再执行一次。这里规定未来业务库 occurrence registry 的存储与事务合同，不借用 push_intents.version，不修改 Task3 的独立 SQL，也不宣称注册表已经实现。

CAS 零行或原状态/版本冲突返回 `schedule.occurrence_conflict`；不追加任何转换事件、不修改状态/版本、不调用 prepare/provider/LLM/sink、不推进游标或产生订单。恢复器重新读取原 occurrence、已提交转换证据及当前 fence，重新评估目标边、窗口与输入；若别的请求已完成同一转换则只返回原事实，只有仍有合法下一步时才构造新的 expected_version 请求，禁止原请求盲重试。陈旧 generation/owner 分别沿用 `activation.generation_conflict` / `activation.owner_conflict`，同样零写作用。version 是快照和并发控制材料，不参与 schedule_occurrence_id，启动 catch-up 与正常 due 仍只合并同一身份。

## 调度身份（PROPOSED）

[Q:16] [Q:33] [Q:74] [Q:86]

| 规则 | 函数 | 有序材料 | 排除材料 | 依据 |
| --- | --- | --- | --- | --- |
| ScheduleOccurrence | SHA256CanonicalTuple | schema_version,namespace,unit_id,producer_id,schedule_or_trigger_id,calendar_id,business_date,occurrence_family,occurrence_key,completion_owner,source_contract_id | wall_clock_tick,phase_epic,activation_generation,version,expected_version,build,payload_sha256,rendered_sha256,evidence_sha256 | [Q:16] [Q:86] |

`schema_version=ScheduleOccurrence/v1` 使用本 RFC 的 canonical tuple 编码及 SHA-256。activation generation 是执行 fence；重启或晋级不改变同一业务 occurrence 的身份。calendar date、采样时间、analytics 时间和消息 receipt 时间不能替代 business date。盘前、集合竞价、盘中、盘后只做规划/观察 Epic，不持有 occurrence、游标或完成状态；不同 producer/owner/occurrence 不因同一 phase 或一分钟合并。

## 调度生命周期（PROPOSED）

[Q:28] [Q:86]。所有边使用 ScheduleOccurrenceTransitionRequest，并无条件遵守「调度版本与转换提交」的 ScheduleVersionRule::v1，保留原身份和转换证据；状态闭集为 `Expected / Eligible / Prepared / Closed / Missed / Deferred / BlockedOnInput`。`Prepared` 仅表示准备结果已经持久化，不代表发送。通知最终化仍使用 Task2/Task3 合同。

| 起点 | 终点 | 权威 | 窗口与版本条件 | 持久事实 | 禁止副作用 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Expected | Eligible | MarketSession | WindowOpen+CurrentVersion+ReadyGate | EligibilityEvent | ProviderOrSend | schedule.window_open | [Q:28] |
| Eligible | Prepared | PhysicalOwner | WindowOpen+CurrentFence+VersionCAS | FrozenDecisionOrIntentRef | DispatchBeforeCommit | intent.created | [Q:76] |
| Prepared | Closed | CompletionPolicy | BoundScheduleCloseProposal+VersionCAS | ScheduleClosureEvent | InferNotificationCursor | schedule.occurrence_closed | [Q:86] |
| Expected | Missed | MarketSession | WindowExpired+NoCatchUp+VersionCAS | MissedEvent | StalePrepareOrSend | schedule.window_expired | [Q:28] |
| Eligible | Missed | MarketSession | WindowExpired+NoCatchUp+VersionCAS | MissedEvent | StalePrepareOrSend | schedule.window_expired | [Q:28] |
| Expected | Deferred | MarketSession | NextEligibleSession+VersionCAS | DeferredEvent+NextEligibilityRef | PrepareOrSend | schedule.deferred | [Q:28] |
| Eligible | Deferred | MarketSession | NextEligibleSession+VersionCAS | DeferredEvent+NextEligibilityRef | PrepareOrSend | schedule.deferred | [Q:28] |
| Expected | BlockedOnInput | SourceContract | UnavailableEvidence+VersionCAS | InputBlockEvent | EmptyAsNoDataOrPollPermanentGap | input.source_unavailable | [Q:30] |
| Eligible | BlockedOnInput | SourceContract | UnavailableEvidence+VersionCAS | InputBlockEvent | EmptyAsNoDataOrPollPermanentGap | input.source_unavailable | [Q:30] |
| BlockedOnInput | Eligible | SourceContract+MarketSession | RecoveryEvent+WindowOpen+ReadyGate+VersionCAS | InputRecoveryEvent | InventProducerOrIdentity | input.source_recovered | [Q:30] |
| BlockedOnInput | Missed | MarketSession | WindowExpired+NoCatchUp+VersionCAS | MissedEvent | StalePrepareOrSend | schedule.window_expired | [Q:86] |
| BlockedOnInput | Deferred | MarketSession | NextEligibleSession+VersionCAS | DeferredEvent+NextEligibilityRef | PrepareOrSend | schedule.deferred | [Q:86] |
| Deferred | Eligible | MarketSession | NextEligibilityReached+ReadyGate+VersionCAS | DeferredRecoveryEvent | InventProducerOrIdentity | schedule.window_open | [Q:86] |

`Deferred` 保存原 occurrence 及下一 eligible session 引用，下一窗口由 authority 重新授予资格并作 `Deferred→Eligible` CAS；不制造同一业务事件的新身份。

## 调度恢复策略（PROPOSED）

[Q:10] [Q:28] [Q:30] [Q:86]

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| ExpireWithoutCatchUp | NewOccurrence | ExpiredMeansMissedNoSend | [Q:28] |
| schema_version | ScheduleOccurrence | ScheduleOccurrence/v1 | [Q:16] |
| SameBusinessDayBeforeDeadline | NewOccurrence | SameBusinessDateAndBeforeWindowEndOnly | [Q:28] |
| DeferToNextEligibleSession | NewOccurrence | PreserveIdentityAndLinkNextSession | [Q:86] |
| RecoverPersistedOnly | ExistingIntentOrDecision | OriginalIdentityAndBytesNoPrepareNoWindowOverride | [Q:10] |
| coalesce | Tick+StartupCatchUp+NormalDue | SameScheduleOccurrenceIdOnly | [Q:16] |
| non_trading_day | SessionBound | NoOccurrence | [Q:28] |
| non_trading_reason | schedule.not_trading_day | EvaluationOnlyNoOccurrenceNoNoDataOrDisabledIntent | [Q:28] |
| independent_trigger | CatalogSessionIndependentEventOrManual | AuthorityBusinessDateRequired | [Q:28] |
| INACTIVE | CatalogMetadata | NoTimerNoProducer | [Q:28] |
| STARVED | ExistingProducer | PreserveStateUntilProductAndInputApproval | [Q:30] |
| OPT-IN | ExistingProducer | PreserveStateUntilExplicitProductApproval | [Q:30] |

非 INACTIVE 定时 producer 注册日程定义，event producer 注册 trigger/readiness；注册本身不运行 producer。启动先恢复既有 intent/outbox/durable decision，以原 identity、原字节和原 authority 引用继续恢复；不能重新 provider/LLM/render，不能把当前窗口许可冒充新 occurrence。时窗过期只禁止新工作，不禁止原 intent 的 reconciliation。NoData/Disabled 的时段关闭由 CompletionPolicy 决定，不增加 Accepted，也不推进通知游标。

## 类型：OperationalReadinessSnapshot（PROPOSED）

创建者：独立 readiness evaluator，读取 namespace、authority、schema、manifest 与 producer 合同证据。消费者：health/readiness probe、部署门禁、CLI 和观测投影。[Q:12] [Q:54] [Q:101]

| 字段 | 类型 | 不变量 | 规范化 |
| --- | --- | --- | --- |
| snapshot_id | Sha256 | canonical snapshot 哈希；不含自身 | 派生且排除自身 |
| captured_at | UtcMicros | 本次观测时间 | 纳入 |
| business_date | Date | 交易日历 authority | 纳入 |
| build_commit | GitSha40 | 当前制品 | 纳入 |
| activation_generation | u64 | 当前已审计 generation | 纳入 |
| scope | ReadinessScope | Core、Producer 或 Occurrence 的 typed ID | 纳入 |
| status | ReadinessStatus | Ready/CoreUnready/ProducerUnready/BlockedOnInput 闭集 | 纳入 |
| reason | ReasonCode | 非 Ready 必须为对应稳定原因 | 纳入 |
| dependency_refs | Vec<DependencyRef> | 合同版本与能力证据 | 纳入 |
| affected_unit_ids | Vec<UnitId> | 非 Ready 为明确受影响集合；Core 包含全部启用 Unit | 纳入 |
| affected_producer_ids | Vec<ProducerId> | 非 Ready 为明确受影响集合 | 纳入 |
| recovery_event_id | RecoveryEventId | 可查询的恢复跟踪事件；未恢复标 Pending | 纳入 |
| evidence_refs | Vec<EvidenceRef> | 类型、受保护 URI、SHA-256、来源及版本 | 纳入 |
| liveness | bool | 与 deployment_ready 分离 | 纳入 |
| deployment_ready | bool | 下表判定，不从日志推断 | 纳入 |
| exit_disposition | ExitDisposition | Continue 或受控非零退出 | 纳入 |

## 运行就绪判定（PROPOSED）

[Q:12] [Q:54] [Q:101]

| 状态 | 范围 | 权威 | 存活 | 部署就绪 | 退出处置 | 恢复事件 | ReasonCode | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Ready | EvaluatedScope | OperationalReadinessSnapshot | true | true | Continue | ReadyObserved | activation.ready | [Q:12] |
| CoreUnready | SharedPrerequisites | OperationalReadinessSnapshot | UntilControlledExit | false | StartupNonzeroOrStopNewAndRecoverIsolateThenNonzero | CoreDependenciesRestored | activation.core_unready | [Q:12] |
| ProducerUnready | AffectedProducers | OperationalReadinessSnapshot | true | false | IsolateAffectedContinueOthers | ProducerContractRestored | activation.producer_unready | [Q:12] |
| BlockedOnInput | KnownOccurrence | OperationalReadinessSnapshot | true | true | ContinueWithoutOccurrenceWork | InputEvidenceRestored | input.source_unavailable | [Q:30] |

Core 前提包括共享 namespace、durable、audit、typed authority、schema、manifest。启动前失败返回非零；运行中失败先停止新 occurrence、保留恢复与隔离，再受控非零退出。ProducerUnready 不阻断其他就绪 producer，但部署 readiness 失败并生成独立 operational alert。

BlockedOnInput 默认不使全局 deployment readiness 失败，前提是不存在其他 Core/ProducerUnready；这是已知 occurrence 的来源证据阻断。ACTIVE producer 的 source/schedule/presentation/policy 合同本身缺失必须升级 ProducerUnready。永久缺能力不定时空转、不用空 Vec 冒充 NoData、不创建 producer；显式 capability/version recovery 事件才能重新评估。

## 就绪查询与恢复合同（PROPOSED）

[Q:12] [Q:54]

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| authority | Health+Readiness+CLI | SameOperationalReadinessSnapshot | [Q:54] |
| query_effects | AllQueries | NoProviderNoSinkNoTransition | [Q:54] |
| log_pager | Logs+OptionalPager | ProjectionOnlyNeverReadinessAuthority | [Q:54] |
| non_ready | CoreUnready+ProducerUnready+BlockedOnInput | StableReasonAffectedSetsQueryableRecoveryEvent | [Q:101] |
| missing_active_contract | Source+Schedule+Presentation+Policy | EscalateProducerUnready | [Q:12] |
| input.source_unready | RegisteredContractOccurrenceEvidenceUnavailable | BlockedOnInput | [Q:12] |
| activation.producer_unready | ActiveProducerContractMissing | ProducerUnready | [Q:12] |
| alert | ProducerUnready | IndependentOperationalAlert | [Q:12] |
| recovery | AllScopes | AppendPendingOrRecoveredEventThenNewSnapshot | [Q:54] |

部署输出包含 snapshot hash、generation、build、受影响 ID、ReasonCode、恢复事件及 `push_total/ready/conditional/compat/inactive/schedule_unreachable/producer_missing/source_missing/presentation_missing/durable_policy_missing` 计数。这些是同一 snapshot 的投影，不用日志出现与否决定 readiness。恢复事件记录前后 snapshot 引用、旧/新依赖版本、认证事件来源与时间，只有重新校验通过才恢复工作。

## 物理所有权与晋级合同（PROPOSED）

[Q:13] [Q:16] [Q:31] [Q:79] [Q:80] [Q:81] [Q:82] [Q:98]

复用「激活转换」表的 `Disabled→Shadow→Active→Draining→Disabled` 与 rollback 新 generation/CAS；不建立第二张 activation 状态机、不修改 DDL。manifest 为期望，promotion journal 为已执行事实，启动必须核对二者。

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| common_fence | LegacyAndNewSchedulerProducerDispatcherFinalizer | unit_id,generation,manifest_sha256,physical_owner | [Q:13] [Q:79] |
| authorization | EveryActor | CurrentGateAndFenceRequired | [Q:98] |
| Shadow | ShadowActorPhysicalOwner | None | [Q:13] |
| Active | NewOccurrence | ManifestOwnerOnly | [Q:31] |
| Draining | NewOccurrenceAndPrepare | ForbiddenPreserveAuthorityFinalizerReconcilerQuarantine | [Q:18] |
| Disabled | PersistedFacts | PreserveAndFenceOldOwnerAgainstResend | [Q:82] |
| daily_limit | NonEmergencyOwnerChangingPromotion | OneUnitPerBusinessDate | [Q:36] |
| no_quota | ShadowOrNoOwnerChangeDeployment | DoesNotConsumeDailyPromotion | [Q:24] |
| emergency_rollback | AnyTime | NewGenerationCASAppendJournalBlockLaterPromotionToday | [Q:80] [Q:99] |
| parallel_shadow | MultipleUnits | ExactlyOneOwnerPerOccurrence | [Q:24] |
| rollback_compatibility | LogicalOrFoundationCompatibleNMinusOne | PreserveAcceptedPendingFactsAndFences | [Q:82] |
| quota_transaction | ActivationDB | BEGIN IMMEDIATE | [Q:36] [Q:80] |
| quota_calendar | AuthorityBusinessDate | CalendarBoundUTCStartInclusiveEndExclusive | [Q:36] |
| quota_query | AllUnitsPromotionJournalOccurredAt | RejectAnyActivateOrRollbackInBusinessDateInterval | [Q:36] [Q:99] |
| quota_apply | SameImmediateTransaction | RevalidateGenerationThenAppendManifestAndJournalCommit | [Q:80] [Q:81] |
| quota_authority | MemoryLockOrLogs | NeverSufficient | [Q:98] |

2026-09-08 所有权范围澄清：Q13 的 shadow 是**新路径**，不是整个 Unit 无 owner。
初始 Disabled 通过已认证批准证据登记实际既有 owner，保持它原本获准的生产范围；
进入 Shadow 不变更该 owner 或生产准入，新路径仅共享 facts 做纯比较。
旧/新副作用 actor 均使用当前代的同一 common_fence；不能另设 legacy 权限真相，
也不能继续用旧代 token。`physical_owner` 保存实际负责身份，None 不能授予任何 actor 权限。

Disabled 的准入不是仅由状态名推导：Initialize 的既有生产范围来自受认证证据；
EnterShadow 保留前代准入；Activate 在当前批准与 fence 下授予目标范围；Drain/Disable
关闭新 occurrence 和 prepare、保留当前负责人的恢复职责。因此排空后的 Disabled 或
其后 Shadow 不会自动重启 legacy。Rollback 仍写新代，按精确历史目标推导其 owner
及原准入范围，并要求本次批准明确授权恢复该范围；旧目标、日志和 actor 字符串均不续权。
这只是从不可变已执行历史派生准入，不新增状态表/可变标记，不改变冻结 DDL。
未登记、缺 journal、来源/实物未认证、确无 owner 或未覆盖旧 binary fence 时不授予权限。
只有真正无既有生产和恢复责任的 Unit 才可经批准表达无 owner；有 pending 不能据此丢弃恢复职责。
依据、替代方案与待实现证明见 [W16 合同裁决](activation-contract-decisions-2026-09-08.md)。
[Q:13] [Q:17] [Q:18] [Q:24] [Q:31] [Q:52] [Q:80] [Q:98] [Q:99]

旧缓存、旧 binary、非空 actor 或 manifest 单独存在均不授权发送。物理 owner 变化必须先 fence 旧 actor，审计最新 generation/journal 后才能授予新 owner；不能以重启创建逃逸 identity。Draining 的原稳定 intent 由当前执行 fence 保护的恢复职责继续处理，外部 Accepted 不可撤销，Uncertain 不盲重发。每日名额是全体 Unit 共用的交易日约束：activation DB 用 `BEGIN IMMEDIATE` 串行，按 catalog 绑定的交易日历 authority business-date 所对应 UTC 半开区间查询全部 Unit 的 journal `occurred_at`，任何 `Activate` 或当日 `Rollback` 都拒绝后续 promote；再重验 generation，写 manifest+journal 并提交同一事务。rollback 不受名额限制但写入新 generation/journal。不能靠内存锁或日志；现有 DDL 已有 occurred_at，区间及 calendar/version 必须绑定批准证据，不能改用 receipt 或本机日期。现有 SQL 的逐 Unit generation 约束不足以单独证明这个跨 Unit 上限；运行时实现必须另交验证证据。

## 风险波次顺序（PROPOSED）

[Q:44] [Q:16] [Q:36]。这十行是有序风险波次，不是十个已冻结原子 Unit。Task5 映射精确 catalog Unit；owner 边界拆分可同 rank，但仍逐 Unit、逐交易日，操作员记录同 rank 内顺序。PaperBuy/Watchdog 不改变此顺序。

| 顺序 | 波次 | 晋级规则 | 依据 |
| --- | --- | --- | --- |
| 1 | CLI report typed BestEffort result | OneUnitPerBusinessDate | [Q:44] |
| 2 | 09:05 chain | OneUnitPerBusinessDate | [Q:44] |
| 3 | 15:30 chain | OneUnitPerBusinessDate | [Q:44] |
| 4 | AttributionDaily | OneUnitPerBusinessDate | [Q:44] |
| 5 | G5bAttribution | OneUnitPerBusinessDate | [Q:44] |
| 6 | 15:05 snapshot occurrence | OneUnitPerBusinessDate | [Q:44] |
| 7 | CandidateBoard + CandidateInvalidated | OneUnitPerBusinessDate | [Q:44] |
| 8 | LimitBoards | OneUnitPerBusinessDate | [Q:44] |
| 9 | ReviewTask result semantics | OneUnitPerBusinessDate | [Q:44] |
| 10 | PaperReview-Starved conformance | OneUnitPerBusinessDate | [Q:44] [Q:30] |

## 影子精确比较（PROPOSED）

[Q:19] [Q:33] [Q:40] [Q:83]

| 比较项 | 输入约束 | 判等规则 | 差异处置 | 依据 |
| --- | --- | --- | --- | --- |
| RunContext | SameInstance | ExactCapturedBusinessContext | BlockUnit:shadow.semantic_diff | [Q:33] |
| PreparedFacts | SameImmutableInstance | IncludingCapturedModelOutputs | BlockUnit:shadow.semantic_diff | [Q:33] |
| JobDecision | SharedFacts | ExactVariantAndAllFields | BlockUnit:shadow.semantic_diff | [Q:83] |
| SemanticProjection.sha256 | SharedFacts | ExactSha256 | BlockUnit:shadow.semantic_diff | [Q:19] |
| PreparedPush.rendered_sha256 | SharedFacts | ExactSha256AndBytes | BlockUnit:shadow.semantic_diff | [Q:19] |
| ReasonCode | SharedFacts | ExactCode | BlockUnit:shadow.semantic_diff | [Q:83] |
| completion_proposal | SharedFacts | ExactScheduleAndCursorProposal | BlockUnit:shadow.semantic_diff | [Q:86] |
| exclusions | ComparisonOnly | attempt_id,latency,diagnostic_timestamp | NoOtherExclusions | [Q:40] |

## 影子副作用（PROPOSED）

[Q:34] [Q:83]。由一次外部采集得到 PreparedFacts，old/new 仅进行纯 prepare/project，禁止 shadow 自行重复取数。副作用端口使用可计数拒绝 capability，保存结构化零调用证明；没有看到日志不能证明零副作用。任一尝试产生 `shadow.side_effect_attempted` 并阻断 Unit。

| 副作用 | 许可 | 失败处置 | 依据 |
| --- | --- | --- | --- |
| provider_second_call | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| llm_recompute | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| business_db_write | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| durable_db_write | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| cursor_advance | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| candidate_watchlist_outcome | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| paper_order_fill | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |
| transport_send | Forbidden | BlockUnit:shadow.side_effect_attempted | [Q:83] |

## 操作员请求与输出（PROPOSED）

[Q:45] [Q:47] [Q:99]。以下为可审计 CLI 的结构化 wire 合同；命令变更由应用校验及事务执行，任何 apply 禁止直接编辑 SQLite。

| 方向 | 字段 | 类型 | 不变量 | 依据 |
| --- | --- | --- | --- | --- |
| Request | command_id | Sha256 | StableCommandIdentity | [Q:45] |
| Request | command | OperatorCommand | inspect/reconcile/resolve-uncertain/promote/rollback | [Q:45] |
| Request | target | TypedTargetRef | TargetTypeAndExactId | [Q:45] |
| Request | expected_version | u64 | CurrentVersionCAS | [Q:77] |
| Request | expected_generation | u64 | CurrentGenerationCAS | [Q:80] |
| Request | dry_run | bool | RequiredForEveryCommand | [Q:45] |
| Request | authenticated_operator_ref | AuthenticatedOperatorRef | HostOrServiceIdentityAndProductionAllowlist | [Q:47] |
| Request | reason | ReasonCode | StableNamespacedCode | [Q:101] |
| Request | evidence_refs | Vec<TypedEvidenceRef> | ProtectedURIAndSHA256AndTypeAndVersion | [Q:55] |
| Request | requested_at | UtcMicros | CapturedRequestTime | [Q:45] |
| Response | decision | OperatorDecision | Inspected/Planned/Applied/Refused | [Q:45] |
| Response | before_refs | Vec<TypedStateRef> | ExactBeforeSnapshot | [Q:45] |
| Response | after_refs | Vec<TypedStateRef> | AppliedOrExplicitlyProjected | [Q:45] |
| Response | affected_rows | u64 | ZeroForDryRunRefusalOrInspect | [Q:45] |
| Response | mutation_journal_event_ref | Option<MutationEventRef> | AppliedMutationOnlyNullForInspectDryRunRefusal | [Q:81] |
| Response | operator_audit_event_ref | OperatorAuditEventRef | IndependentControlPlaneEnvelopeReference | [Q:45] [Q:55] |
| Response | refusal_reason | Option<ReasonCode> | RequiredWhenRefused | [Q:101] |
| Response | snapshot_sha256 | Sha256 | CanonicalResponseAndEvidenceBinding | [Q:45] |

## 操作员命令（PROPOSED）

[Q:45] [Q:46] [Q:99]

| 命令 | 目标 | 最小证据 | 允许行为 | 拒绝原因 | 演练 | 依据 |
| --- | --- | --- | --- | --- | --- | --- |
| inspect | UnitOrIntentOrDecision | AuthenticatedIdentity+TargetRef | ReadOnlySnapshot | operator.unauthorized | SupportedNoWrites | [Q:45] |
| reconcile | IntentOrUnit | CurrentVersionFence+AuthorityRefs | DeterministicIdempotentRecoveryOnly | intent.expected_version_conflict | SupportedNoWrites | [Q:45] |
| resolve-uncertain | Decision | PriorInspect+ExactTerminalBinding+CurrentFenceVersion+ExternalEvidenceHash | AppendManualDispositionNeverRewriteReceipt | operator.resolution_conflict | SupportedNoWrites | [Q:46] |
| promote | Unit | CurrentManifest+SixFreshGates+WaveRank+DailyJournal+OnlineApproval | OneOwnerCASAppendJournal | activation.generation_conflict | SupportedNoWrites | [Q:99] |
| rollback | Unit | CompatibleRollbackTarget+CurrentFenceVersion+AuthenticatedRollbackPermission | NewGenerationCASAppendJournalPreserveAcceptedPending | activation.generation_conflict | SupportedNoWrites | [Q:80] |

`reconcile` 只做已被原 intent/authority 授权的确定性恢复，不创建 occurrence，不盲发 Uncertain。人工处置先 inspect 同一 decision，再复核 exact terminal binding、current fence/current version；只追加 ManualConfirmedAccepted 或 ManualConfirmedNotDelivered，保留原 transport receipt。
后者须先取得独立 operator audit 引用与哈希，再按不投递终态合同 CAS 收敛 NotDelivered；
不撤销接受历史、不授权重发，失败指标保留。命令权限之外还需核对 namespace、Unit、合同和最小证据，不以非空 actor 授权。

## 操作员权限（PROPOSED）

[Q:43] [Q:47] [Q:99]

| 规则 | 认证与审批 | 规范值 | 依据 |
| --- | --- | --- | --- |
| SingleControl | AuthenticatedOnlineUserOrProductionAllowlistedOperator | V1BaselineOneMayApproveAndExecute | [Q:43] [Q:47] |
| DualControl | ExternalUnitOrOrganizationPolicy | DistinctAuthenticatedPreparerAndApproverCannotDowngrade | [Q:99] |
| emergency_rollback | AuthenticatedOperatorWithRollbackPermission | SingleOperatorAllowedAuditRequired | [Q:99] |
| Codex | EvidenceAndCommandPreparation | NeverProductionApproverOrExecutor | [Q:99] |
| dry_run_and_refusal | EveryCommand | NoDBNoJournalNoOwnerChangeNoProviderNoLLMNoSinkNoOrder | [Q:45] |
| unauthorized | MissingOrFreeTextIdentity | Refuse:operator.unauthorized | [Q:47] |
| invalid_evidence | MissingInvalidOrUnboundEvidence | Refuse:operator.evidence_invalid | [Q:45] |
| stale_version | ExpectedVersionConflict | Refuse:intent.expected_version_conflict | [Q:77] |
| stale_generation | ExpectedGenerationConflict | Refuse:activation.generation_conflict | [Q:80] |
| binding_conflict | TerminalOrOwnerBindingMismatch | Refuse:operator.resolution_conflict | [Q:46] |
| refusal_audit | OperatorAuditEnvelope | IndependentControlPlaneAuditSinkOnly | [Q:45] [Q:55] |
| audit_envelope | AuthenticatedIdentityOrUnauthenticatedMarker | CommandHashTimeReasonEvidenceHashSnapshotHash | [Q:45] [Q:55] |
| dry_run_refusal_storage | BusinessDurableActivationDBAndPromotionJournal | NoWrites | [Q:45] |

SingleControl 是 v1 人力基线：一名在线用户或 production allowlist 指定操作员可批准并执行；Codex 只准备证据和命令。外部策略要求 DualControl 时不能降级，preparer/approver 必须为不同认证身份。紧急 rollback 按显式 rollback 权限由单个认证操作员执行并留痕。在线批准必须绑定当前 command、Unit、manifest、generation、窗口和证据 hash，旧批准不覆盖新请求。

dry-run 输出标记 Planned 的 before/after 投影，实际 affected_rows=0。拒绝返回 Refused、稳定 ReasonCode、当前快照 hash 和最小审计 envelope（含认证身份或未认证标记、请求 hash、时间与拒绝原因）；授权控制面的独立审计接收该 envelope，命令拒绝/dry-run 本身不写 DB/journal、不切 owner，不调用 provider/LLM/sink/order。这样拒绝可留审计且不会走成功 apply 的 journal 写入路径。

## 证据保留类别（PROPOSED）

[Q:48] [Q:55] [Q:88]。监管依据：[v18.1 §审计盲区](../v18.x/v18.1-strategic-gap-analysis.md) 的 `>5年` 与 [v19.0 §边界](../v19.x/v19.0-operational-clarity-design.md) 的 WORM/Object-Lock 边界。此处定义未来合同，不宣称外部 WORM 已部署或本地 SQLite 已具备该能力。

| 类别 | 起算条件 | 最低策略 | 存储与清理 | 依据 |
| --- | --- | --- | --- | --- |
| NonTerminal | UntilVerifiedTerminal | NeverAutoDelete | PreserveIncludingUncertainAndResolutionRequired | [Q:88] |
| MigrationEvidence | TerminalAndProductionVerified | AtLeast90DaysAfterBoth | CleanupEligibilityAllRequired | [Q:48] |
| DeliveryAuditRegulatory | ApplicableRegulatoryStart | StrictlyGreaterThanFiveYears | ExternalWORMOrObjectLockNeverRewrite | [Q:88] |
| ModelDecisionTrade | ApplicablePolicyStart | StrictestRegulatoryModelTradeSourcePolicy | NoUnifiedFiveYearMaximum | [Q:88] |

NotDelivered 属于可重验的已处置终态，不是 Production Verified 成功证明；必须等关联 Unit
获得独立 Production Verified 证据才满足迁移保留起算条件。未解决 ResolutionRequired 仍属非终态。
迁移证据在合法终态与 Production Verified 两个条件均满足后起算至少 90 天；更严格策略继续优先。监管投递审计严格大于五年，不能固定为 1825 天，闰年、法规起算及更严格模型/交易来源规则必须正确处理，五年不是统一保留上限。

## 清理资格与安全（PROPOSED）

[Q:48] [Q:53] [Q:55] [Q:88]

| 条件 | 规范值 | 依据 |
| --- | --- | --- |
| terminal_binding | VerifiedExactLegalBindingRequired | [Q:88] |
| transition_journal_audit | AllLinkedIntegrityVerifiedRequired | [Q:81] |
| retention_expiry | StrictestApplicablePolicyExpiredRequired | [Q:88] |
| legal_hold | AbsentRequired | [Q:88] |
| disclosure | MinimumMetadataProtectedURIHashOnly | [Q:55] |
| backup_integrity | IndependentBackupsAndIntegrityEvidenceRequired | [Q:53] |
| nonterminal_uncertain_resolution | NeverAutoDelete | [Q:88] |
| worm_mutation | Forbidden | [Q:88] |
| secrets_and_unnecessary_content | NoKeysCookiesWebhookURLUnnecessaryBodyOrPositions | [Q:55] |

清理 eligibility 是全部条件的逻辑与，缺一项即拒绝；不能因 MigrationEvidence 的 90 天已到期越过法规留存。清理只追加处置审计，不改写 WORM；business/durable 独立备份和校验，不声称跨库原子快照。

## 通用晋级门禁（PROPOSED）

[Q:35] [Q:37] [Q:84] [Q:100]

| 门禁 | 输入 | 通过证据 | 失败原因 | 阻断晋级 | 依据 |
| --- | --- | --- | --- | --- | --- |
| unit | CurrentUnitBuildCatalogContracts | TypedDecisionsExactBindingsAndAllBranches | input.evidence_invalid | true | [Q:84] |
| failure | TestNamespaceFaultMatrix | RejectionUncertainIsolationNoFalseCompletion | transport.uncertain | true | [Q:84] |
| crash | SevenStepCommitBoundaries | OriginalIdentityBytesRecoveryNoLostIntent | intent.transition_conflict | true | [Q:84] |
| shadow | SharedContextFactsAndEffectCounters | ExactCompareZeroForbiddenEffects | shadow.semantic_diff | true | [Q:84] |
| dedup | SameDecisionReplayAndBusinessRevisions | NoSecondSendNoOverDedupExactReceipt | intent.payload_conflict | true | [Q:84] |
| rollback | PendingAcceptedUncertainAndOldOwner | NewGenerationJournalFenceNoResend | activation.owner_conflict | true | [Q:84] |

每个 Unit 每次晋级前重新跑全部六门禁，证据绑定 Unit、build、manifest/schema/catalog/template/source-contract 哈希、generation、测试时间及样本范围；不能继承别的 Unit 或旧 build 的绿灯。任意 semantic diff、重复发送、未解释游标推进、超龄积压、未解决 Uncertain、CoreUnready、ProducerUnready 或双库不一致均阻断 promotion。

高频路径观察至少一个完整 eligible session，条件允许时不少于三个 occurrence；每日/低频路径确定性回放加一次自然 occurrence 或明确授权灰盒；Emergency/业务副作用路径至少两个 eligible session。每个启用的 authoritative channel 必须有真实 typed Accepted 和同 decision AlreadyDelivered/无第二次发送证明，不能用日志、COMPAT 或人工接受替代。Accepted→Finalized 目标两个 reconcile 周期、硬上限五分钟；未收敛不得继续晋级。Uncertain 保留 Q39 的 Emergency 1/15 分钟、Important 5 分钟/4 小时、Info/Research 下一 eligible session 前人工处置标准，不假设 Codex 自动值守。

## 业务验收样本（PROPOSED）

[Q:100]。所有行都是要求，尚未执行/通过样本验收。基线为冻结 `07781bf` 的 65-kind/52-Unit；样本来自已有分析报告，未连接或复制 `data/**`、消息正文、数据库或私人数据。下表来源路径相对仓库根；F01--F12 与 §9 仅引用已分析结论。

| 样本 | 基线身份 | 必须证明 | 禁止结论 | 门禁 | 来源路径 | 章节定位 | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| historical_backfill_2026-08-31 | CURRENT_65_KIND:TomorrowWatch+PositionReview | OriginalBusinessDateStableDecisionExactReplay | SendDateEqualsBusinessDateOrLogMeansAccepted | unit,dedup,crash | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F01/§9 | [Q:100] [unit:MU-review-r07] [unit:MU-review-r11] |
| n02_receipt_time | CURRENT_65_KIND:NewsFlashAggregated | ReceiptAcceptedAtSeparateFromSourceAnalyticsTime | WindowTimeMeansDeliveryTimeOrMissingMeansLoss | unit,shadow | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F01/§F10 | [Q:100] [unit:MU-news-flash-aggregate] |
| g5b_test_namespace | CURRENT_65_KIND:G5bAttribution | TestRecordsRejectedBeforeProductionProviderLLMSink | SymbolPrefixReplacesNamespaceOrDeletePollution | failure,unit,shadow | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F02 | [Q:100] [unit:MU-g5b-attribution] |
| news_ai_cross_batch | CURRENT_65_KIND:NewsToIdea;producer=news-ai-same-tick | BatchOnlyNoNewNotificationValidRevisionRemainsDistinct | CountReductionTargetOrAssessmentHashMeansContentRevision | dedup,shadow,crash | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F03 | [Q:100] [producer:news-ai-same-tick] [unit:MU-news-ai] |
| paper_sell_254_2026-09-01 | CURRENT_65_KIND:PaperSell | EachFillIntentTracePartialFailureRecoveryNoNewOrderSegmentLatency | 254MeansDuplicateOrFileLatencyMeansAcceptedLatency | unit,failure,crash,dedup | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F04 | [Q:100] [unit:MU-paper-sell] |
| attribution_g5b_sink_fail | CURRENT_65_KIND:AttributionDaily+G5bAttribution | SavedResultsReuseNoLLMRecomputeNoEarlyCursor | AnalysisSavedMeansDelivered | failure,crash,shadow | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F05 | [Q:100] [unit:MU-attribution-daily] [unit:MU-g5b-attribution] |
| r03_blocked_input | CURRENT_65_KIND:IndustryChain;ReviewTask=R03 | FixedContractGapVisibleNoPollingNoNewProducer | BlameUserSnapshotOrEmptyAsNoData | unit,failure | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F07 | [Q:100] [unit:MU-review-r03-auto] [unit:MU-review-r03-manual] |
| r08_retryability | CURRENT_65_KIND:EventCalendar;ReviewTask=R08 | NonretryableEvidencePreservedUntilCapabilityRecovery | StringMeansRetryableOrDropCFFEXRequirement | unit,failure | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F08 | [Q:100] [unit:MU-review-r08] |
| no_data_disabled_uncertain | CURRENT_65_KIND_SCOPE:AllApplicableUnits | SeparateScheduleNotificationManualCounts | EmptyMeansNoDataOrDisabledMeansAcceptedOrBlindResend | unit,failure,dedup | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §9 | [Q:100] [Q:86] |
| cross_db_conflict_rollback | CURRENT_65_KIND_SCOPE:AllApplicableUnits | CASConflictResolutionRequiredNewGenerationPreserveAccepted | OverwriteConflictOrUndoExternalAccepted | crash,rollback,dedup | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §9 | [Q:100] [Q:80] |

08-31 历史补推涉及 08-26 PositionReview、08-28 TomorrowWatch/PositionReview，引用 MU-review-r07/MU-review-r11；不制造 ReviewBackfill kind。NewsAI 是业务路径名，对应 NewsToIdea 的 news-ai-same-tick producer。batch 是 lineage；相同事实只换 batch 不新增通知，跨目标/受众/交易日及明确有效修订仍分别验证，保留原 assessment/audit 的严格留存。PaperSell 的 254 条只作逐 fill 追踪样本，不以数量推断重复；恢复通知不得重跑模拟成交。R03 的固定合同缺口与用户快照缺失不是同一根因，若 ACTIVE 合同缺失按 ProducerUnready 隔离，不能永久伪装 occurrence 级 BlockedOnInput。

## 非基线回放样本（PROPOSED）

[Q:30] [Q:44] [Q:100]

| 样本 | 状态 | 允许证明 | 禁止结论 | 门禁 | 来源路径 | 章节定位 | 依据 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| paper_buy_29_2026-09-04 | NON_BASELINE_REPLAY_ONLY | DesignReplayFilledVersusNotFilled | NoCatalogUnitNoBaselineCapabilityNoProducerActivationNoWaveChange | unit,crash,dedup | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F04 | [Q:30] [Q:100] |
| watchdog_nonbaseline | NON_BASELINE_REPLAY_ONLY | DesignReplayLateStartupSlowReviewMissingRegistrationAlertFailure | NoCatalogUnitNoBaselineCapabilityNoProducerActivationNoWaveChange | unit,failure,crash | docs/push-system/comprehensive-reanalysis-2026-09-05.md | §F06 | [Q:30] [Q:100] |

09-04 的 29 条 PaperBuy 与 Watchdog 仅为 67-kind 根工作树的非基线反例：不创建 catalog Unit、不证明当前隔离源码或部署能力、不激活 producer、不改变 Q44 顺序。Watchdog 回放区分 expected/progress/attempt/Accepted，晚启动、慢 review、无注册和 alert sink 失败不能用 fired 位抹平；该设计不能反推基线已具备哨兵。

## 故障环境与验收边界（PROPOSED）

[Q:49] [Q:99] [Q:100]

| 环境 | 许可 | 禁止行为 | 依据 |
| --- | --- | --- | --- |
| Test | RejectionUncertainCrashReplayRollbackManualResolution | ProductionNamespaceAccess | [Q:49] |
| Production | ApprovedNormalTypedReceiptAndSameDecisionIdempotentReplay | DisconnectKillDatabaseOrderOrManufactureFault | [Q:49] [Q:99] |

本 RFC 继续为 PROVISIONAL。这些门禁只定义后续实施/验收合同，文档 validator 通过不代表运行时接线、生产晋级、WORM 部署或真实样本通过。Task4 不改 Rust/Cargo、SQL、目录、冻结来源或任何运行数据库；Unit 精确排期/风险波次映射由 Task5 承接，独立双轴复核由 Controller 安排。

## 不投递终态合同（PROPOSED）

[Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97]

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| state | BusinessIntentState | NotDelivered；独立业务终态，不增加 durable 的十四态或 DeliveryResult 分支 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| entry | AwaitingAuthority/ResolutionRequired | 仅 Ready；后者最近进入隔离必须来自同 intent/decision 的 AwaitingAuthority+transport.uncertain；历史不得已有 AwaitingFinalizer/Completed | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| verification | VerifiedTerminalRef | 已认证操作员与生产 allowlist；私有 authority 重查精确 ManualConfirmedNotDelivered 绑定、外部证据哈希与独立 operator audit | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| commit | ExpectedVersionCAS | 同一业务事务 CAS 与追加 event；包含 terminal_ref_id、terminal_disposition、terminal_decision_id、binding SHA、operator audit 引用与 SHA | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| storage_trust | SQLite/Application | SQL 验证边、原 decision、字段组与格式；应用验证身份认证、authority 真实性、hash 内容与事务包装 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| cursor | NotDelivered | 永不推进通知游标；不授权重发；不作为 Accepted 或 ProductionVerified 成功样本 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| recovery | CommitAckLost | 重查原 terminal 后只补本地终态事务；已提交时返回原 event；禁止再次外发 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| gate | ResolvedButFailed | 解除未解决 ResolutionRequired/Uncertain 阻断；failure 门禁及失败指标仍保留，不自动批准晋级 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| retention | TerminalEvidence | 关联原 intent、decision、transition、operator audit；满足最严格保留及清理资格才可清理，不因已处置立即删除 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |
| rollback | AcceptedHistory | NotDelivered 无离开边；AwaitingFinalizer/Completed 及其隔离历史不得撤销为不投递 | [Q:39] [Q:45] [Q:46] [Q:78] [Q:87] [Q:88] [Q:97] |

## 运行里程碑（PROPOSED）

[Q:5] [Q:15] [Q:17] [Q:25] [Q:42] [Q:44] [Q:50]

| 标识 | 名称 | 前置条件 | 完成条件 | 本批状态 | 依据 |
| --- | --- | --- | --- | --- | --- |
| FoundationReady | Foundation Ready | TypedResultThenFinalizerReconcilerAndCompatibleSchemas | NoOwnerChange+EachAuthorityControlledAcceptedAndSameDecisionAlreadyDeliveredNoSecondSend+TestRestore+ParallelIsolation | NotAttained | [Q:5] [Q:15] [Q:17] [Q:25] [Q:42] [Q:44] [Q:50] |
| P0ProductionVerified | P0 Production Verified | FoundationReady+Q44ApprovedNonNullWaveUnits | EachApplicableP0UnitFreshSixGatesAndAuthorizedNaturalOrLowFrequencyEvidence+RequiredChannelReceipts | NotAttained | [Q:5] [Q:15] [Q:17] [Q:25] [Q:42] [Q:44] [Q:50] |
| ArchitectureReleaseCandidate | Architecture Release Candidate | FoundationReady+All52UnitsImplemented | 42OwnerChangingAnd10ConformanceOnlyCodeContractsTestsComplete+NoImplicitActivation+DeletionGatesBeforeCleanup | NotAttained | [Q:5] [Q:15] [Q:17] [Q:25] [Q:42] [Q:44] [Q:50] |
| ProgramProductionVerified | Program Production Verified | ArchitectureReleaseCandidate+All52UnitsVerified | CompleteCatalog+AllUnitsComplete+NoAccidentalActivationUncertainBacklogDuplicateReceiptGap+TailCleanup+FreshEvidence | NotAttained | [Q:5] [Q:15] [Q:17] [Q:25] [Q:42] [Q:44] [Q:50] |

## 运行退出验收（PROPOSED）

[Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69]

| 规则 | 适用范围 | 规范值 | 依据 |
| --- | --- | --- | --- |
| unit_inventory | CurrentCatalog | 52 Units；42 owner-changing 与 10 conformance-only；不按 PushKind/count 推导 owner | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| nullable_waves | Q44 | 只使用 WBS 当前非空批准波次；null 不代表遗漏、不自动赋予第十一波或生产授权；全部 52 Unit 仍在项目退出范围 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| foundation_greybox | EachAuthoritativeRequiredChannel | 受控 Accepted 与同一 decision 的 AlreadyDelivered/no-second-send；弱 COMPAT 或人工接受不得替代 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| unit_greybox | EachUnit | 自然 occurrence；低频仅经批准确定性灰盒；conformance-only 验证原 owner 而非虚构接管 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| parallel_tests | DefaultParallelCI | 默认并行无无法解释失败；进程全局状态测试隔离或显式强制串行并记录范围，禁止隐匿失败 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| backup_restore | BusinessDBAndDurableDB | 分别备份并记录各自 hash 与边界；在 Test 恢复并对账；不是跨库原子快照 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| old_path_delete | PriorUnit | Accepted、same-decision replay、restart、fault、有效 session、Uncertain 全部门禁通过后，才在后续版本删除旧路径 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| release_pipeline | ReleaseNAndNPlus1 | N 接管当前 Unit；N+1 清理前一 Unit 并可晋级下一 Unit；最后单独完成 tail cleanup | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| program_exit | All52Units | 目录完整、所有 Unit 完成、无意外激活、未解决 Uncertain、陈旧 backlog、duplicate、receipt 缺口；清理结束并有 fresh evidence | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| failure_retained | NotDelivered | 已处置不等于发送成功；保留失败指标与 failure 门禁，不能冲抵成功回执缺口 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| publication_boundary | ImplementationReady | 仅文档发布资格；与四级 runtime milestone 正交，本批四级均未达到；后续 HTML/CI 发布不证明生产 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| historical_estimate | Q41 | 36–69 工程人日与 7–10 交易周是目录冻结前暂估；现行机器 WBS 重新建立基线，保留完整范围 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |
| priority | Q22Q26 | 先修假成功、过早状态与语义分裂；C0–C6 仅能力标签；Foundation→垂直 Unit→尾部清理 | [Q:20] [Q:21] [Q:22] [Q:23] [Q:26] [Q:29] [Q:41] [Q:44] [Q:50] [Q:51] [Q:53] [Q:69] |

## 外部兼容（PROPOSED）

[Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27]

| 表面 | 保持项 | 内部边界 | 验收 | 破坏性变化 | 依据 |
| --- | --- | --- | --- | --- | --- |
| cli | Invocation+Arguments+ExitStatus+Output | BoolToTypedResultViaCompatibilityAdapter | ExistingInvocationGoldenArgsExitStdoutStderr+InvalidArgsAndModeMatrix | SeparateVersionedDecision+Unit+Acceptance | [Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27] |
| config | Key+Default+Scope | PreserveExistingParsingDefaultsAndNamespace | ExistingKeyDefaultScopeGolden+MissingInvalidCrossScopeCases | SeparateVersionedDecision+Unit+Acceptance | [Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27] |
| subscription | Subscription+Audience+RequiredChannels | PreserveRoutingAndCompletionPolicy | SameSubscriptionAudienceChannelSet+MissingRequiredChannelRefusal | SeparateVersionedDecision+Unit+Acceptance | [Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27] |
| template | TemplateId+Version+RenderedBytes | InfrastructureMigrationNeverChangesWordingOrTemplate | SameFactsIdVersionExactFirstRenderedBytes+ReplayNoRerender | SeparateVersionedDecision+Unit+Acceptance | [Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27] |
| authority | COMPATWeakEvidence | NeverTransportAcceptedOrVerifiedTerminalRefOrCursorAdvance | WeakOutcomeMatrixRejectsAuthorityUpgradeAndCursorMutation | SeparateVersionedDecision+Unit+Acceptance | [Q:4] [Q:7] [Q:10] [Q:15] [Q:19] [Q:27] |

不投递处置在 durable 库封存后、业务事务前中断：重启先重查同一 decision 的
ManualConfirmedNotDelivered，再以 expected-version 与当前 fence 追加本地 NotDelivered；
独立 operator audit 必须已持久化且可重验。若 audit 写入成功而业务提交失败，仅留下可查询
处置审计，不冒充业务完成；若提交确认丢失，只读既有终态与事件。不得补发，不得让旧
AwaitingFinalizer/Completed 或其隔离记录逆转接受历史。schedule 关闭仍独立按策略，
NotDelivered 自身不推进通知游标。[Q:39] [Q:45] [Q:53] [Q:78] [Q:87] [Q:97]

以上兼容验收是未来每个 Unit 的行为合同，需从迁移前冻结外部基线取得 golden 样本，
而不是本批虚构某个 CLI、配置或模板已完成运行验证。内部可把 bool 改为 typed result，
外部 invocation/arguments/exit/output、配置默认与作用域、订阅路由和原始消息字节均由
adapter 保留；异常修复若必须破坏旧外部语义，应独立批准并版本化，不藏在基础设施迁移中。
文档 Implementation-Ready 尚未达到；之后离线 HTML/CI 发布也不授予任何 runtime milestone。
[Q:4] [Q:42] [Q:69] [Q:105]

## 裁决追踪（PROPOSED）

[Q:63]。下表逐行保留 Q1–Q55 的冻结选择；约束摘要保留历史覆盖关系，
不改写冻结 grill 字节。落点是当前稳定章节名，不沿用旧稿失效数字节号。
依据列解析为冻结 evidence/Unit/producer，或本 RFC 规范验收、门禁、里程碑、
明确的未来文档发布边界。publication:ImplementationReady 只指后续文档发布；
不是已经完成的实现证据，也不能替代 runtime 验收。[Q:59] [Q:66] [Q:69]

| Q | 冻结选择 | 约束摘要 | 规范落点 | 证据或验收引用 |
| --- | --- | --- | --- | --- |
| 1 | B | 覆盖活跃及高风险路径的 C0--C6；不激活 24 个 INACTIVE 类型。 24 是历史口径；当前冻结目录为 22 个 INACTIVE，仍全部不激活。 | 范围与事实权限 | [gate:unit] |
| 2 | A | 只有获得可验证且已接受的权威结果后，业务通知游标才可推进。 | 终态完成合同（PROPOSED） | [gate:dedup] |
| 3 | C | 初始按严重级别区分 Uncertain 的处理方式；Q9 随后禁止所有严重级别盲目重发。 | 通用晋级门禁（PROPOSED） | [gate:failure] |
| 4 | A | 内部布尔结果迁移为类型化合同期间，保持外部 CLI、配置、订阅和模板兼容。 | 外部兼容（PROPOSED） | [acceptance:cli] |
| 5 | B | 代码完成仅达到 Release Candidate；必须取得受控真实传输证据，才能达到 Production Verified。 | 运行里程碑（PROPOSED） | [milestone:ArchitectureReleaseCandidate] |
| 6 | B | 迁移 P0 调用方前先建立最小结果合同，再增加 finalizer 与 reconciliation。 | 运行里程碑（PROPOSED） | [milestone:FoundationReady] |
| 7 | B | 只有能提供类型化、可验证回执的传输通道才具有权威性；其他通道维持 COMPAT/BestEffort。 | 外部兼容（PROPOSED） | [acceptance:authority] |
| 8 | A | 业务库负责通用通知意图与最终化记录；durable DB 保存下游尝试和回执。 | 跨库恢复顺序（PROPOSED） | [gate:crash] |
| 9 | B | 任何严重级别都不得盲目重发 Uncertain；严重级别只影响检查和升级速度。 | 通用晋级门禁（PROPOSED） | [gate:failure] |
| 10 | B | 重放时保持稳定身份和首次渲染字节不变；新的 occurrence 可使用新模板版本。 | 业务 outbox 字节恢复合同（PROPOSED） | [gate:dedup] |
| 11 | C | 从小版本开始；Q16/Q31 将发布边界从 PushKind 细化为原子 MigrationUnit/晋级。 | 物理所有权与晋级合同（PROPOSED） | [gate:rollback] |
| 12 | B | CoreUnready 阻断生产；ProducerUnready 隔离单个 producer，同时使部署就绪检查失败并告警。 | 运行就绪判定（PROPOSED） | [gate:unit] |
| 13 | B | 新路径先进入 shadow；同一 occurrence 必须且只能有一个物理 owner。 | 物理所有权与晋级合同（PROPOSED） | [gate:shadow] |
| 14 | A | 业务迁移只做增量、向前兼容变更；回滚时保留表和证据。 | 持久化条件组与兼容守卫（PROPOSED） | [gate:rollback] |
| 15 | B | 每个权威传输通道都要提供受控 Accepted，以及同一 decision 的 AlreadyDelivered/未二次发送证据。 | 运行退出验收（PROPOSED） | [milestone:FoundationReady] |
| 16 | B | 原子迁移身份是 producer + occurrence family + completion owner，而不是 PushKind 或源文件。 | 范围与事实权限 | [unit:MU-p01] |
| 17 | A | 任何 Unit 晋级前，先发布一个不改变物理 owner 的 Foundation。 | 运行里程碑（PROPOSED） | [milestone:FoundationReady] |
| 18 | B | 回滚时禁用新 producer/scheduler，但在状态收敛前保留 authority、finalizer、reconciler 和隔离栅栏。 | 物理所有权与晋级合同（PROPOSED） | [gate:rollback] |
| 19 | B | shadow 精确匹配包括 audience、kind、occurrence、severity、suppression、policy、evidence，初期还包括字节。 | 影子精确比较（PROPOSED） | [gate:shadow] |
| 20 | B | 只有 Accepted、重放、重启、故障、交易时段和 Uncertain 门禁全部通过后，才能在后续版本删除旧路径。 | 运行退出验收（PROPOSED） | [gate:rollback] |
| 21 | A | 编目所有 ACTIVE、STARVED 和 OPT-IN 路径；强路径只做合规校正，仅迁移不合规接线，INACTIVE 保持禁用。 | 范围与事实权限 | [gate:unit] |
| 22 | C | 优先处理假成功、过早状态变更和语义分裂，再处理 scheduler/template 的用户体验。 | 运行退出验收（PROPOSED） | [milestone:P0ProductionVerified] |
| 23 | B | 采用流水线发布：版本 N 清理 Unit N-1，并可晋级 Unit N；最后保留一个尾部清理版本。 | 运行退出验收（PROPOSED） | [milestone:ProgramProductionVerified] |
| 24 | B | 多个 Unit 可并行 shadow，但每次晋级只能让一个 Unit 获得物理所有权。 | 物理所有权与晋级合同（PROPOSED） | [gate:shadow] |
| 25 | B | Foundation 阶段完成传输灰盒证明；随后每个 Unit 证明其自然 occurrence，或使用经授权的低频灰盒。 | 运行退出验收（PROPOSED） | [milestone:FoundationReady] |
| 26 | B | C0--C6 仅作为能力标签；实施顺序为 Foundation → 垂直 Unit 切片 → 最终清理。 | 运行退出验收（PROPOSED） | [milestone:ProgramProductionVerified] |
| 27 | B | 只设一个应用端口/结果合同；默认使用通用 coordinator；P01/N02 保留为经过一致性测试的专用 authority。 | 适配器一致性合同（PROPOSED） | [evidence:p01-identity] |
| 28 | B | 所有定时且非 INACTIVE 的 producer 注册到 PhaseScheduler；事件驱动 producer 注册 trigger/readiness；INACTIVE 不创建 scheduler。 | 调度身份（PROPOSED） | [gate:unit] |
| 29 | B | 只有精确、机器可读的 MigrationUnit 目录完成后，才冻结排期和估算。 | 运行退出验收（PROPOSED） | [publication:ImplementationReady] |
| 30 | B | 保留 STARVED 和 OPT-IN 状态；恢复输入或激活必须另做产品决策。 | 运行就绪判定（PROPOSED） | [gate:unit] |
| 31 | B | 一个制品可包含多个 disabled/shadow Unit；一次晋级只能变更一个物理 owner。 | 物理所有权与晋级合同（PROPOSED） | [gate:rollback] |
| 32 | A | 只有共享原子 completion owner 的路径才可分组，包括已识别的候选、板块、复盘、大宗交易和财报家族。 | 范围与事实权限 | [gate:unit] |
| 33 | B | 新旧 shadow projection 使用同一份不可变 PreparedFacts，包括已捕获的 LLM 输出。 | 适配器一致性合同（PROPOSED） | [gate:shadow] |
| 34 | B | 拆分 prepare/project/deliver/finalize；shadow 只能执行 prepare/project。 | 影子副作用（PROPOSED） | [gate:shadow] |
| 35 | B | 语义差异、重复发送、无法解释的游标移动、陈旧积压、未解决 Uncertain、CoreUnready 或 DB 不匹配均阻断晋级。 | 通用晋级门禁（PROPOSED） | [gate:failure] |
| 36 | B | 每个交易日最多晋级一个物理 owner；开发和 shadow 工作可并行。 | 物理所有权与晋级合同（PROPOSED） | [milestone:P0ProductionVerified] |
| 37 | B | 按风险观察：高频路径覆盖完整有效时段/样本；低频路径确定性重放；紧急或有副作用的 Unit 观察两个时段。 | 通用晋级门禁（PROPOSED） | [gate:failure] |
| 38 | B | Accepted 到 Finalized 的目标为两个 reconcile 周期，硬上限五分钟；下一次晋级前不得存在超时状态。 | 通用晋级门禁（PROPOSED） | [gate:crash] |
| 39 | B | Emergency 告警/解决 SLA 为 1/15 分钟，Important 为 5 分钟/4 小时，Info/Research 须在下一有效时段前完成或标为 NotDelivered。 | 不投递终态合同（PROPOSED） | [acceptance:not_delivered] |
| 40 | B | shadow 精确比较只排除 attempt ID、延迟和日志时间戳；业务时间取自捕获的 RunContext。 | 影子精确比较（PROPOSED） | [gate:shadow] |
| 41 | A | 保持完整范围，暂估 36--69 工程人日、7--10 个交易周；目录冻结后重新建立基线。 本文以现行机器 WBS 为重建基线，旧暂估不冒充当前总工期。 | 运行退出验收（PROPOSED） | [publication:ImplementationReady] |
| 42 | B | 分开定义 Foundation Ready、P0 Production Verified、Architecture Release Candidate 和 Program Production Verified。 | 运行里程碑（PROPOSED） | [milestone:ProgramProductionVerified] |
| 43 | A | 仅在用户或指定操作员在线时晋级；Codex 提供证据和命令，不进行无人值守裁决。 | 操作员权限（PROPOSED） | [gate:unit] |
| 44 | A | 按已批准的风险顺序迁移 10 个 P0 Unit，从 CLI BestEffort 和链路报告开始。 当前 WBS 细化为十个批准风险波次及 nullable rank，不按旧数量捏造十个 owner。 | 运行退出验收（PROPOSED） | [milestone:P0ProductionVerified] |
| 45 | B | 人工处置使用可审计 CLI，记录决策、结果、已认证操作员、原因和证据；禁止直接修改 DB。 | 操作员请求与输出（PROPOSED） | [acceptance:not_delivered] |
| 46 | B | ManualConfirmedAccepted 与 TransportAccepted 必须区分，前者要求可复核的外部证据及其哈希。 | 权威处置与最终化资格（PROPOSED） | [acceptance:authority] |
| 47 | B | 操作员身份来自已认证主机/服务身份及生产 allowlist，不得使用自由文本。 | 操作员权限（PROPOSED） | [acceptance:not_delivered] |
| 48 | B | 终态迁移证据至少保留 90 天；非终态证据不得自动清理；更严格策略仍优先。 | 证据保留类别（PROPOSED） | [gate:unit] |
| 49 | B | 测试命名空间覆盖拒绝、Uncertain、接受后崩溃、幂等 finalizer、重放、回滚和人工处置；禁止破坏性生产故障注入。 | 故障环境与验收边界（PROPOSED） | [gate:failure] |
| 50 | B | Program Production Verified 要求目录精确、Unit 全部完成、无意外激活/Uncertain/积压/重复、传输证明完备、完成清理并取得新鲜回执。 | 运行退出验收（PROPOSED） | [milestone:ProgramProductionVerified] |
| 51 | B | 规范 CI 在默认并行模式下不得有无法解释的失败；隔离进程级全局测试，或明确强制串行套件。 | 运行退出验收（PROPOSED） | [milestone:FoundationReady] |
| 52 | B | 由唯一、带 schema 版本的 activation manifest 管理 Disabled/Shadow/Active/Draining，并记录其哈希。 | 激活转换（PROPOSED） | [gate:rollback] |
| 53 | B | 分别备份和校验 business/durable DB，在 Test 环境演练恢复；不得声称存在跨库原子快照。 | 运行退出验收（PROPOSED） | [gate:crash] |
| 54 | B | 运行故障通过结构化本地日志、readiness/health 和可查询 CLI 保持可见；外部分页告警与业务回执相互独立。 | 就绪查询与恢复合同（PROPOSED） | [gate:unit] |
| 55 | B | 人工证据只保存最小元数据、受保护 URI 和内容哈希；不得保存密钥或非必要的消息/投资组合内容。 | 清理资格与安全（PROPOSED） | [acceptance:not_delivered] |

<!-- RFC-WBS-BEGIN -->
## WBS 确定性摘要（PROVISIONAL）

事实源为 [push-system-wbs.v1.json](push-system-wbs.v1.json)。本区间仅为生成视图；修改事实源后运行 render-wbs.rb --write。

PROVISIONAL：规格非实现、非部署、非生产验收。旧 W01--W21 合计 98--142h 仅为历史对照，无法恢复旧逐项表；本表 lineage=reconstructed_2026-09-06，不拟合旧范围。

catalog SHA256：`0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3`。Unit hash 对原catalog对象递归排序键后编码无空格/换行UTF-8 JSON，数组保持原顺序；这是对象快照hash，不是新增运行时身份合同。

### Foundation：恰好 W01--W21

| ID | 工作包 | O/M/P小时 | PERT小时 | 依赖 | 风险 | 验收 |
| --- | --- | --- | --- | --- | --- | --- |
| W01 | 身份、业务日、occurrence 与 source-contract 基础合同 | 8/12/20 | 12.67 |  | high | 同日期不同source-contract不能合并；重启及generation变化不改变 occurrence。 |
| W02 | 应用 DeliveryResult 与现有 durable 类型适配 | 6/10/16 | 10.33 | W01 | high | BestEffort/Partial/NoChannel不得构造强终态，十四种durable状态逐一投影。 |
| W03 | CompletionPolicy、ReasonCode 与 RetryPolicy | 8/13/22 | 13.67 | W01,W02 | high | NoData/Disabled/Uncertain分别判定调度、通知游标与人工处置；退避保留typed reason。 |
| W04 | RunContext 与 PreparedFacts 单次取数 | 6/11/18 | 11.33 | W01,W03 | high | old/new共享同一PreparedFacts与捕获模型输出，第二次外部采集有拒绝计数。 |
| W05 | SemanticProjection、PreparedPush 与 exact bytes 绑定 | 7/12/19 | 12.33 | W02,W04 | high | 同facts重建语义hash相同；首次渲染字节封存，重放不重新渲染。 |
| W06 | catalog/Unit/completion-owner 运行时注册表 | 5/8/13 | 8.33 | W01,W03 | high | 65 kind/102 producer/52 Unit双向闭合，枚举外入口保留独立注册。 |
| W07 | business intent schema 与迁移 | 9/15/24 | 15.5 | W01,W03,W06 | high | DDL重复应用不改原字节，拒绝版本冲突且保留原库与非终态事实。 |
| W08 | append-only transition/outbox | 8/14/23 | 14.5 | W07 | high | 每个intent版本连续且前驱hash相连，commit和outbox之间逐边界崩溃可恢复。 |
| W09 | VerifiedTerminalRef 构造与重验证 | 8/13/21 | 13.5 | W02,W05,W08 | high | 构造和finalize时重验authority/decision/bytes/subject全部绑定，弱audit永不冒充receipt。 |
| W10 | 通用 finalizer 与业务 CAS | 10/16/26 | 16.67 | W03,W08,W09 | critical | CAS冲突进入ResolutionRequired；Accepted不可撤销，重复finalize只推进一次。 |
| W11 | reconciler、lease/fence 与启动恢复 | 10/17/28 | 17.67 | W08,W09,W10 | high | 恢复所有原业务日既存intent；过期lease重取fence，Uncertain不得盲重发。 |
| W12 | transport authority port 与通用 adapter | 7/12/20 | 12.5 | W02,W05,W09 | high | 逐required channel记录typed结果；Partial拒绝游标，强receipt保留exact bytes。 |
| W13 | P01/N02 专用 conformance adapter | 9/14/25 | 15 | W03,W09,W12 | high | P01同日claim不分render-mode；N02 accepted-window独立于N01 critical quota。 |
| W14 | PhaseScheduler 与 occurrence catch-up | 8/13/22 | 13.67 | W01,W03,W06,W11 | high | 窗口半开区间和原业务日catch-up可回放；closed occurrence不可重开。 |
| W15 | readiness/operational snapshot 与 deploy probe | 6/10/17 | 10.5 | W06,W11,W14 | high | Core/Producer/Occurrence依赖缺失分别判级，恢复事件绑定前后snapshot及依赖版本。 |
| W16 | activation manifest、generation CAS 与 owner fence | 10/16/27 | 16.83 | W06,W08,W11,W12 | critical | 同事务验证全Unit当日journal和generation；legacy/new四类actor共同fence。 |
| W17 | shadow harness 与 typed diff | 7/11/19 | 11.67 | W04,W05,W12,W16 | high | exact typed diff只排除attempt/latency/diagnostic time；八副作用端口证明零调用。 |
| W18 | operator inspect/reconcile/resolve/promote/rollback | 9/15/25 | 15.67 | W10,W11,W15,W16 | high | dry-run/refusal不写DB/journal；SingleControl允许认证一人，外部DualControl不得降级。 |
| W19 | 指标、SLA、保留期与安全审计 | 6/10/18 | 10.67 | W08,W10,W15 | high | Accepted两周期目标/五分钟上限；未决永不自动删，监管审计严格大于五年。 |
| W20 | fault/replay/dedup/rollback 回归 harness | 10/17/29 | 17.83 | W11,W13,W14,W17,W18,W19 | high | 测试namespace覆盖七步崩溃、拒绝/不确定、rollback；PaperBuy/Watchdog仅nonbaseline回放。 |
| W21 | 发布编排、N/N-1 兼容和逐 Unit/tail-cleanup 门禁工具 | 6/10/17 | 10.5 | W16,W18,W19,W20 | high | 只交付编排/门禁工具；逐Unit cutover和tail cleanup人工工时在Unit行，90天及更严留存另等。 |

W21只交付发布编排与清理门禁工具；逐Unit cutover准备、验证及tail-cleanup资格核验在Unit估算中。保留期届满后的生产删除不属于本次规格交付。

### 四 Epic / 52 Unit

| Epic | 关联Unit数 | 关联PERT小时 |
| --- | --- | --- |
| 盘前 | 13 | 141.32 |
| 集合竞价 | 10 | 110.82 |
| 盘中 | 28 | 294.48 |
| 盘后 | 34 | 355.15 |

跨Epic Unit在关联行重复展示，不能累加Epic行作为总数；去重后 52 Unit，547.65 小时。

### Q44 十波映射

| rank | CatalogUnit | physical-owner晋级session | 观察session |
| --- | --- | --- | --- |
| 1 | MU-cli-single, MU-cli-summary | 2.0 | 2.0 |
| 2 | MU-chain-preopen | 1.0 | 2.0 |
| 3 | MU-chain-post-close | 1.0 | 2.0 |
| 4 | MU-attribution-daily | 1.0 | 2.0 |
| 5 | MU-g5b-attribution | 1.0 | 2.0 |
| 6 | MU-intraday-market | 1.0 | 2.0 |
| 7 | MU-auction-candidates | 1.0 | 2.0 |
| 8 | MU-limit-boards | 1.0 | 2.0 |
| 9 | MU-review-a10, MU-review-r04, MU-review-r07, MU-review-r08, MU-review-r09, MU-review-r11, MU-review-r13 | 7.0 | 14.0 |
| 10 | MU-paper-review-daily, MU-paper-review-noon | 0.0 | 0.0 |

同rank不代表有内部先后顺序：仍逐Unit逐交易日，同波内顺序须操作员另批。其他Unit rank=null，未经新批准不能追加为第十一波或按流量排序。rank1仅含default CLI单股/汇总的typed BestEffort结果；CLI产业链报告的历史批准范围有歧义，MU-cli-chain保持rank=null，纳入波次需要另行产品裁决；replay-force独立。rank6覆盖15:05所属共享owner的四入口；rank9仅七个ACTIVE ReviewTask，R03三owner rank=null。rank10是PaperReview保持STARVED的conformance，不授予物理owner。

### 可复算时间与首批关键路径

O/M/P包含实现、评审和修复。逐行 PERT=round-half-up((O+4M+P)/6,2)，总计仅加保存的逐行PERT。Foundation 281.34h + Unit 547.65h = 828.99h / 8 = 103.62工程日。

单开发者串行；缓冲只在总PERT上应用一次 20%=165.8h。工程区间为baseline 828.99h至含缓冲 994.79h，即 103.62至124.35个8小时工程日。外部等待/交易观察/同一风险不重复进入工时。

工程DAG最长依赖路径：W01 → W02 → W03 → W06 → W07 → W08 → W09 → W10 → W11 → W14 → W15 → W18 → W20 → W21 → MU-paper-sell = 208.01h；这不是单开发者总历时。完整资源串行顺序存于JSON，可检查每条依赖。首批工程是全部Foundation加rank1--3的 MU-chain-post-close, MU-chain-preopen, MU-cli-single, MU-cli-summary，共318.66h（无缓冲）。

交易独立计算：42个owner-changing Unit，42次晋级 + 76次独立观察 = 118个串行eligible session；单日全局最多晋级一个Unit，下限42个晋级交易日。观察按每Unit晋级后串行保守场景；高风险/业务副作用至少两观察session，纯shadow不占名额。首批rank1--3至少10个session，同rank排列须另批。

自然日场景从假设周一开始且不承诺日期：ceil(124.35)=125工程工作日 + 118交易session + 63外部等待工作日 = 306个串行业务日；只排周末时 7*floor((N-1)/5)+(N-1)%5+1 = 428自然日。该保守无重叠场景须另加交易所休市、人工批准和真实样本延迟，上限为null；非承诺，亦非把交易日直接当自然日。STARVED/OPT-IN激活及至少90天/更严留存届满等待均不在此场景，未排序Unit须新批准。

近期08-31--09-04仅影响设计、回放与预修复，门禁引用RFC既有业务样本；PaperBuy/Watchdog仅nonbaseline反例，不新增第53/54 Unit。

### 完整 52 Unit 附录

#### MU-announcement — 公告路由

owner：news_dedup.key=annroute:{observed_date}:{source}:{external_id}；独立 L4(announcement,source_fact_event_id,空 sub_kind)。Epic：盘中/盘前/盘后；producer：news-announcement。快照SHA：`9902e0d2725c97f50d26250447c361fbaf278b45f113157bd3560ce70f515fc2`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/9/16h；PERT=9.5h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：公告到达窗口，跨日source-id重现；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：两层非原子claim与L4拆开故障覆盖。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：source/external_id与observed_date需稳定绑定，claim释放后崩溃回放不得重复公告，L4不能替代receipt。

#### MU-p01 — 盘前新闻P01

owner：business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}。Epic：盘前；producer：p01-compensation, p01-scheduled, startup-resume-preopen-news-hot。快照SHA：`973018c0000cf29fec4af83d629c9026672d440d810d0144ccb55cef69e351bf`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/12/21h；PERT=12.67h；风险=high；外部等待=2工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W06, W07, W08, W09, W10, W11, W12, W13, W14, W16, W17, W18, W19, W20, W21。日历：交易日盘前及补偿窗口；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：专用authority和补偿/恢复三入口共同封口。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：scheduler与compensation同business-date claim；render mode变化不增claim；原P01 envelope重验accepted绑定。；盘前新闻P01：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-d01 — 公告选股D01

owner：D01_LAST_PUSH&#91;code:name&#93;；COOLDOWN_TABLE(NewsToIdea,空 code)；L4 无冷却（PerTicket 缺 code）。Epic：盘中；producer：d01-announcement, d01-manual。快照SHA：`dc3b9e522c6ce52bb5ee0d96de577837929aed44d33d03a36329be53219aa1ff`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/8/15h；PERT=8.67h；风险=medium；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：公告触发与显式manual窗口；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：不能把不存在的L4 owner实现为新增去重权限。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：公告与manual同code:name memo，空code的PerTicket确无L4冷却，banner拒绝不得推进memo。

#### MU-news-catalyst — 新闻催化I02

owner：L4(news_catalyst,空 code,空 sub_kind)；模板 COOLDOWN_TABLE(NewsCatalyst,空 code)。Epic：盘中；producer：catalyst-announcement, catalyst-manual。快照SHA：`949b777a3f783077f464d31c059d54094a08ab80aaea656d5029bce203411081`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=4/7/12h；PERT=7.33h；风险=medium；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：公告eligible session与manual；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：共享下游冷却的两入口需要独立触发回放。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：I02空code模板/L4冷却独立于D01 memo；manual失败不消耗新公告完成资格。

#### MU-news-ai — NewsAI业务通知

owner：news_ai_delivery_event(delivery_identity_sha256=assessment_id,reservation,state)；assessment=provider+batch_id+item_id+target_code+analysis_version hash。Epic：盘中/集合竞价；producer：news-ai-same-tick。快照SHA：`896064314d797194f4135082a98cc4a92ba4a3dbc75fdbd5431727a04eafbb86`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=9/15/26h；PERT=15.83h；风险=high；外部等待=2工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：同tick及跨batch eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：近期样本优先预修复；batch是lineage，不能为降消息数抹掉修订。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F03跨batch相同事实不新增通知；目标/受众/业务日及有效修订分别验证；保留assessment/delivery/prediction链。

#### MU-news-flash-aggregate — N02新闻聚合窗口

owner：NewsFlash authority accepted-window(business_date,window) / window_state&#91;index&#93;；reservation_identity_sha256+attempt_ordinal。Epic：盘中/盘后；producer：news-flash-aggregate。快照SHA：`f18feb9c210232349e986b911e1ea363c94777b1defc1ed395fbdb8e7c76d072`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/13/23h；PERT=13.83h；风险=high；外部等待=2工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W06, W07, W08, W09, W10, W11, W12, W13, W14, W16, W17, W18, W19, W20, W21。日历：每个聚合窗口完整session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：专用accepted-window authority需窗口失败/恢复证明。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F01/F10 receipt accepted_at与source analytics time分开；window reservation/attempt匹配，N01 critical quota不得借此激活。

#### MU-earnings-beat — 业绩超预期OPT-IN

owner：L4(earnings_beat,source_fact_event_id(earnings:{code}:{report_date}),空 sub_kind)。Epic：盘后；producer：earnings-beat。快照SHA：`ffb0284b313b642e0ffc8e2ac20e47480d10b16d201efb7c7f54508883431834`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=3/6/10h；PERT=6.17h；风险=medium；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：未启用时仅测试隔离conformance，启用另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：provider后配置门不授权自动开启功能。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：Beat保持OPT-IN未激活；缺分类配置/来源依赖可见，零消息不是通过；正向event.kind不得完成Miss。

#### MU-earnings-miss — 业绩低预期OPT-IN

owner：L4(earnings_miss,source_fact_event_id(earnings:{code}:{report_date}),空 sub_kind)。Epic：盘后；producer：earnings-miss。快照SHA：`6261c1595dbc226fd1371026ad1a13e0610d79ed672d13104417b79420aa9066`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=3/5/9h；PERT=5.33h；风险=medium；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：负向报告样本回放，启用另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：共享earnings轮询不构成共享通知owner。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：Miss保持OPT-IN未激活；缺配置/来源依赖可见，零消息不是通过；负向分类独立Beat并保留report_date。

#### MU-analyst — 分析师上调

owner：L4(analyst_upgrade,source_fact_event_id(analyst:{code}:{broker}:{report_id}),空 sub_kind)。Epic：盘后；producer：analyst-upgrade。快照SHA：`b8d35b3cdb814a0aec8195e0ff8879cb57279a30a808bb94204dd07dde52653d`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=4/7/13h；PERT=7.5h；风险=medium；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：研报自然到达或批准灰盒；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：经纪商维度与报告修订的过度去重风险。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：code+broker+report_id稳定事件identity；AnalystStateStore观察推进不等于L4通知Accepted。

#### MU-auction-volume — 竞价放量

owner：monitor_loop.auction_vol_notified&#91;session,code&#93;；独立 L4(auction_volume,空 code,空 sub_kind)。Epic：集合竞价；producer：auction-volume。快照SHA：`4744895bba638a1295b300003568ec2576d9838283da63e267945a238c7ef19d`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/18h；PERT=10.67h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：集合竞价完整eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：两次读取股票集合可能不一致，先修PreparedFacts。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：外层new_items与dispatcher snapshot.items须同批绑定；逐票有限正价格/量比，失败不得insert通知set。

#### MU-auction-candidates — 竞价候选主卡与失效

owner：monitor_loop.post_close_candidates_notified&#91;session&#93;；candidate_board_snapshot&#91;{date}&#93;.jsonl 最末 code 集（双层非原子推进链）。Epic：集合竞价；producer：auction-repush, candidate-board, candidate-invalidated。快照SHA：`5725bd51f9b8178e8988353b39e2e1d09ea862a10403498df569e58192467c5c`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/14/24h；PERT=14.67h；风险=high；外部等待=1工作日；owner change=true；rank=7；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：集合竞价主卡及后续失效session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：Q44 rank7按真实共享推进链迁移，不能按三kind拆owner。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：A02/主卡/失效共双bool和末尾code快照推进链；部分失败不提前持久快照，三kind独立receipt可追踪。

#### MU-virtual-watch — 虚拟观察STARVED

owner：L4(virtual_watch,空 code,空 sub_kind)；共享 monitor_loop.virtual_observation vector / virtual_snapshot_persisted&#91;session&#93;。Epic：盘中/集合竞价；producer：virtual-watch-confirm, virtual-watch-pilot。快照SHA：`99a6deef2592d3119027d1e59e6ab5471886be070d997504bb00a13a00e5cb68`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=4/7/12h；PERT=7.33h；风险=high；外部等待=2工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：隔离pilot/confirm样本，来源恢复另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：业务vector和snapshot并非通知完成状态。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED未激活；空post_close/vector依赖缺失可见，零消息不是通过；confirm补价不得改变pilot资格来伪造成功。

#### MU-paper-trade — 模拟交易终态通知

owner：counted decision(PaperTrade,Ticket,terminal_transition_id,source fingerprint,subject,policy,rendered hash)。Epic：集合竞价；producer：paper-trade-terminal, startup-resume-paper-trade。快照SHA：`db43f13207c8ff2d4655384ffa4731aec8885dc1d048ef5beb309de016a35051`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/14/25h；PERT=14.83h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：交易终态出现后至少两eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：业务副作用需成交事实与通知崩溃分别追踪。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：PaperTradeTerminalBindingV1 terminal_transition_id与通知decision绑定；回放只恢复通知不再成交，不与PaperSell code/day/Filled合并。；模拟交易终态通知：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-limit-boards — 连板三展示形态

owner：monitor_loop.board_notified&#91;session,code&#93;；L4(limit_boards,空 code,空 sub_kind)。Epic：盘中；producer：limit-boards-first, limit-boards-second, limit-boards-third-plus。快照SHA：`1b14ee102bfe471333ceef536e63de838cda2ed5bcbdbf21974a3558398e1848`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/9/17h；PERT=9.67h；风险=high；外部等待=1工作日；owner change=true；rank=8；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：盘中连板变化完整session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank8需要反证共享空code冷却吞掉其他票。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：first/second/third-plus共享code set；发送前insert的故障不得永久吞票；token不制造子类dedup identity。

#### MU-holding-plan — 持仓计划

owner：counted decision(HoldingPlan,Ticket,holding-plan:{date}:{code},source fingerprint,subject,policy,rendered hash)。Epic：盘中；producer：holding-plan-manual, holding-plan-periodic, startup-resume-holding-plan。快照SHA：`e7b8832319b67e4aa899f882895f3a92205c8427c6cd5c1fcf9f1ae50156b3c9`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/11/20h；PERT=11.83h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：持仓发生及定时扫描session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：多入口与immutable binding组合需要修订样本。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：定时/manual同holding-plan:date:code只有source/rendered hash一致才同decision；修订计划不可被日级展示名压掉。；持仓计划：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-t0 — T0建议

owner：counted decision(T0Advice,Ticket,T0PlanDecisionBindingV1.decision_id(),source fingerprint,subject,policy,rendered hash)。Epic：盘中；producer：startup-resume-t0-advice, t0-advice。快照SHA：`a3281d95ce7fb9e63b5fd57b94122e6498622e5eb6b166db1b6e790c0c9629d1`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/12/22h；PERT=13h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：T0 eligible时段至少两session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：交易建议identity专用，错误重放有业务风险。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：T0PlanDecisionBindingV1 canonical decision与HoldingPlan独立；last_t0_scan不是完成游标；建议恢复不可重下单。；T0建议：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-close-call — 尾盘操作提醒

owner：counted decision(CloseCall,Ticket,close-call:{date}:{code},source fingerprint,subject,policy,rendered hash)。Epic：盘中；producer：close-call, startup-resume-close-call。快照SHA：`8acc5a1f5cdedf6fe69ad401a5e2b43e4c7ffcc4bbc3c040999328643d467299`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/18h；PERT=10.67h；风险=high；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：尾盘窗口与次日startup回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：外层bool和durable计数不能互相覆盖。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：close-call:date:code绑定原decision；close_call_pushed不替代receipt；窗口过期后只恢复既存发送事实。；尾盘操作提醒：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-industry-intraday — 盘中产业链I03

owner：L4(industry_chain_intraday,空 code,空 sub_kind)；COOLDOWN_TABLE(IndustryChainIntraday,空 code)。Epic：盘中；producer：industry-chain-manual, industry-chain-periodic。快照SHA：`bddd8edca0d370ff23b34d00d4c4c8464e6198517da01e7fd94362e179822dfb`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/8/14h；PERT=8.5h；风险=medium；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：盘中周期与manual自然occurrence；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：同业务分析名称涉及三个不同owner家族。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：periodic/manual共享I03空code冷却；last_industry_chain_intraday仅调度，不能共享R03或enum外chain的完成。

#### MU-intraday-market — 盘中市场与15:05快照

owner：L4(intraday_market,空 code,空 sub_kind)。Epic：盘中/盘前/盘后；producer：market-manual-i01, market-preopen-probe, market-snapshot-warning, market-view-periodic。快照SHA：`c5b00d9ee00a79dc9764b6d2190c5bb58d91e79c1ac250c7c548ea14f958cdd5`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/13/23h；PERT=13.83h；风险=high；外部等待=1工作日；owner change=true；rank=6；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：15:05窗口及另外三入口独立回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank6按catalog整体owner而非只切一条producer。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：15:05 SNAP_REMIND_LAST与market view/preopen probe/manual外门分别保留；共享空code冷却不应让一次bool替四入口封日。

#### MU-sector-top — 强势板块

owner：business_date_once_claims(business_date,SectorTop,None,GLOBAL) → immutable decision / occurrence=sector-top:{date}。Epic：盘中；producer：sector-top, startup-resume-sector-top。快照SHA：`464dd154ba3a3ed4cbdc8b387a7c40aba56c03f741db19fd11fbdeab5e4b11f8`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/9/16h；PERT=9.67h；风险=high；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：每日一次自然板块occurrence；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：共享coordinator不足以共享claim。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：SectorTop日claim只对自身kind完成；last_sector_top和coordinator budget分别验证，不能吞SectorAnomaly。；强势板块：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-sector-anomaly — 板块异动

owner：business_date_once_claims(business_date,SectorAnomaly,None,GLOBAL) → immutable decision / occurrence=sector-anomaly:{date}。Epic：盘中；producer：sector-anomaly, startup-resume-sector-anomaly。快照SHA：`fbc8da70ee1cf01acc5fd4ebfa38cce98cca09107ba689150ad8075405b5808c`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/17h；PERT=10.5h；风险=high；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：异动eligible session与日边界；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：板块内容修订和独立预算分支需对照。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：SectorAnomaly异动source绑定自身日claim；外last_sector_anomaly失败保留重试，恢复不借SectorTop receipt。；板块异动：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-data-mode — 数据模式通知

owner：LATEST_DATA_MODE；DATA_MODE_PENDING_STABLE(mode,since)；DATA_MODE_UNSAFE_REMINDER(fingerprint,external_confirmed_at,heartbeat_at)。Epic：盘中/盘前/盘后；producer：data-mode。快照SHA：`10df8726c0af15198e568396b906a806bd230647ff1b148997185fe3e6c24736`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/12/20h；PERT=12.5h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：模式稳定窗口及不安全heartbeat两session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：健康事件误确认会隐藏来源故障。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：LATEST/PENDING_STABLE/UNSAFE_REMINDER三个状态分开；heartbeat更新绝不伪造external_confirmed_at，fingerprint变化保留新资格。

#### MU-account-mode — 账户模式主通知

owner：account_mode_log&#91;log_id&#93;.pushed（同模式未确认复用 log_id）。Epic：盘中/盘前/盘后；producer：account-mode-main。快照SHA：`2d4260eaa1cfd73f061f0b62140852edb65c3db40d4ad7d5115c69dc86e871ad`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/11/19h；PERT=11.67h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：账户模式变化后两eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：主通知外部结果和hook/banner共同完成条件。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：同模式未确认复用log_id；account_mode_log.pushed须在typed receipt与最终banner刷新条件都满足后CAS。

#### MU-frozen-side — Frozen副通知

owner：L4(market_action_alert,FROZEN,空 sub_kind)；触发资格来自 account_mode_log 新建事实，无副推持久确认列。Epic：盘中/盘前/盘后；producer：account-frozen-side。快照SHA：`ab086af9985264e2d6444c483ed3d8777e37632768b74209f0ed313017e01ba9`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/9/15h；PERT=9.33h；风险=critical；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：Frozen事件两eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：共享kind不能把主副通知合为一个receipt。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：新account_mode_log只触发Frozen副推；主pushed不得完成FROZEN code独立L4通知，失败保留副推intent。

#### MU-order-alert — 订单变化通知

owner：MarketActionState.seen&#91;code&#93;=(action,shares)；L4(market_action_alert,code,空 sub_kind)。Epic：盘中/盘前/盘后；producer：order-update-alert。快照SHA：`9b25b932fcc9df9b07cec62cdeca57917a508f7829fc1f321f9aa9cb6d0ba5ab`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/13/24h；PERT=14h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：真实订单变化样本或授权灰盒，两session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：高风险业务状态先推进链需七步故障追踪。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：EventBus seen(code,action,shares)先变更不应吞通知；订单revision绑定通知intent，重放不能提交或修改订单。

#### MU-paper-sell — 模拟卖出通知

owner：paper_trades(code,direction=sell,status=Filled,date(ts))；L4(paper_sell,code,空 sub_kind)。Epic：盘中/盘后；producer：paper-sell-intraday, paper-sell-post-close。快照SHA：`7b04f33d2d4f56b2bfefbc99f64dc8ed0919b146c81ba98a03b769afad7c3c52`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=10/16/28h；PERT=17h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：盘中/盘后卖出两eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：近期优先回放但不改变Q44，Filled业务去重独立通知。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F04的254条逐fill→intent追踪；部分通知失败恢复不得重跑模拟成交；分段latency不能用文件时间替Accepted时间。

#### MU-snapshot-stale — 账户快照过期提醒

owner：check_snapshot_staleness_and_notify::LAST:SnapshotReminderGate(today,last_confirmed,in_flight)。Epic：盘中/盘前/盘后/集合竞价；producer：snapshot-stale-startup, snapshot-stale-timer。快照SHA：`329470d64fb3428ea29b85fb601e1c33f0418e4fa236ac67df73155f7125f6fd`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/8/14h；PERT=8.5h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：启动与定时提醒跨交易日；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：同static函数预约恢复需要避免假confirmed。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：startup/timer共享SnapshotReminderGate today/in_flight/last_confirmed；拒绝释放预约，和持仓六小时警告分开。

#### MU-attribution-daily — 每日归因

owner：monitor_loop::ATTRIBUTION_LAST_RUN&#91;calendar_date&#93;。Epic：盘后；producer：attribution-daily。快照SHA：`83a1e47f85614639c578dc2958c0a15a32c2fc28f70aa6bfd2e6033801f6a955`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/18h；PERT=10.67h；风险=high；外部等待=1工作日；owner change=true；rank=4；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：每日归因窗口及失败回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank4用独立通知cursor替分析成功bool。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F05保存归因结果后sink失败只重用结果，不再调用LLM；ATTRIBUTION_LAST_RUN不得提前封日且不存在L4冷却owner。

#### MU-g5b-attribution — G5b归因

owner：monitor_loop::G5B_LAST_RUN&#91;calendar_date&#93;。Epic：盘后；producer：g5b-attribution。快照SHA：`5fcd04a289eb3489191fd28eb110e59d9e113741e5152cd5fe88d1289530a26c`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/12/21h；PERT=12.83h；风险=high；外部等待=1工作日；owner change=true；rank=5；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：G5b日窗口与namespace隔离样本；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank5既需namespace拒绝又需无L4路径恢复。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F02 test namespace在production provider/LLM/sink前拒绝；F05复用保存结果，G5B_LAST_RUN不因保存而封口。

#### MU-fixed-order — 盘后定价订单STARVED

owner：monitor_loop.last_post_fixed_order&#91;session&#93;；L4(post_fixed_price_order,code,空 sub_kind)。Epic：盘中/盘后；producer：post-fixed-order。快照SHA：`97cb996100829fa02d754960a89842f5522a8bffac5fb623ba272870e7fcc127`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=4/7/13h；PERT=7.5h；风险=critical；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：仅隔离T14事件样本，源注册另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：无生产注册必须保留阻断事实。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED未激活；TradeEventSource OnceLock未注册依赖可见，零消息不是通过；T14 900s timer不创造消费ack或订单。

#### MU-fixed-fill — 盘后定价成交STARVED

owner：monitor_loop.last_post_fixed_fill&#91;session&#93;；L4(post_fixed_price_fill,code,空 sub_kind)。Epic：盘中/盘后；producer：post-fixed-fill。快照SHA：`45d933a62d3868e45ec76db230ba27ae69fd299cb74c1f776d6070b75debea26`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=5/8/14h；PERT=8.5h；风险=critical；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：仅隔离T15成交样本，源注册另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：同缺源但成交事件identity/资格区别于订单。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED未激活；T15来源依赖缺失可见，零消息不是通过；300s timer按fill事件独立校验，不借T14完成。

#### MU-st-price — ST价格限制批次

owner：monitor_loop.st_price_pushed&#91;session&#93;；L4(st_price_limit_changed,code,空 sub_kind)。Epic：盘中；producer：st-price-limit-batch。快照SHA：`3e10a668bbfb29dbc716336e8e3d775a01c63e605f4c4f66a29ff8086b91d471`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/9/17h；PERT=9.83h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：ST限制变化自然批次或授权灰盒；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：外批次bool吞掉空数据和部分失败。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：零条Ok不能封st_price_pushed；部分逐票Accepted不回滚也不遮蔽失败票，code冷却与批次关闭分开。

#### MU-paper-review-noon — 午间PaperReview STARVED

owner：monitor_loop::NOON_SNAP_LAST&#91;calendar_date&#93;；潜在模板/L4(PaperReview,noon-code,空 sub_kind)。Epic：盘中；producer：paper-review-noon。快照SHA：`945b17aa515e0191c6604ae0055064e1a91c48ee5b598123e3f8e01f7457441b`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=4/6/11h；PERT=6.5h；风险=high；外部等待=2工作日；owner change=false；rank=10；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：午间隔离conformance，来源恢复另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank10 noon外门和daily实际code并非同owner。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED未激活；today结构与来源缺依赖可见，零消息不是通过；NOON_SNAP_LAST不得因受阻bool封日，保留noon-code。

#### MU-review-r04 — R04龙虎榜复盘

owner：business_date_once_claims(business_date,ReviewLhb,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,R04)。Epic：盘后；producer：review-r04-auto, review-r04-backfill, review-r04-manual, startup-resume-review-lhb。快照SHA：`cd5bf3bdd9de820950789aeb4610e33411811276f0007ee1f67d655086cc3efb`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/17h；PERT=10.5h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：盘后自然R04及历史业务日回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：ReviewTask结果语义与同claim多入口结合。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：R04 auto/manual/backfill同原日期ReviewLhb claim；先hydrate再due/attempt，终态Rejected不能标作Delivered。；R04龙虎榜复盘：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r07 — R07明日观察

owner：business_date_once_claims(business_date,TomorrowWatch,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,R07)。Epic：盘后；producer：review-r07-auto, review-r07-backfill, review-r07-manual, startup-resume-tomorrow-watch。快照SHA：`3c6ba5e204dab31c199095c3ef33d3a49621729b36aee8414cf402910db24743`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/11/19h；PERT=11.67h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：盘后R07与历史补推样本；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：近期历史回放优先，发送日不能替原业务日。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F01 08-31补08-28 TomorrowWatch保留08-28业务日与原decision；manual临时audit不得覆盖自动task state。；R07明日观察：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r08 — R08事件日历

owner：durable review occurrence(business_date,EventCalendar,None,GLOBAL,review_task_identity(date,R08)) → Rolling immutable decision。Epic：盘后；producer：review-r08-auto, review-r08-backfill, review-r08-manual, startup-resume-event-calendar。快照SHA：`3175fb884c35aaf43d8dd46950a53c30cf60891463416d5ffd20840f9be04f47`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=8/13/22h；PERT=13.67h；风险=high；外部等待=2工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：事件来源能力恢复及盘后eligible窗口；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：固定source-contract故障与单次等待分开。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F08保留nonretryable证据直至能力恢复；CFFEX需求不删除，Rolling occurrence按原R08业务日恢复，不能靠字符串判断可重试。；R08事件日历：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r09 — R09供应商TopN

owner：business_date_once_claims(business_date,ReviewProviderTopN,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,R09)。Epic：盘后；producer：review-r09-auto, review-r09-backfill, review-r09-manual, startup-resume-review-provider-top-n。快照SHA：`56e156d454d6b399d0e01bc9f92ba26fad986ed92f279cebec8c8719fafc5c53`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/9/16h；PERT=9.67h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：供应商数据到齐后的盘后occurrence；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：来源迟到与业务结果终态的分支组合。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：ReviewProviderTopN auto/manual/backfill共日claim；ExpectedWait/DeferredUntil按时间门复核，永久Failed终态不增通知Accepted。；R09供应商TopN：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r11 — R11持仓复盘

owner：business_date_once_claims(business_date,PositionReview,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,R11)。Epic：盘后；producer：review-r11-auto, review-r11-backfill, review-r11-manual, startup-resume-position-review。快照SHA：`bbe650096450cb78c65a46efd74eb14918ca8e5c74b38172869e7896afbf5c37`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/12/20h；PERT=12.5h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：盘后R11及两历史日期回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：多业务日样本要求逐intent而非单发送日统计。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：F01 08-31补08-26/08-28 PositionReview逐原日期绑定；新的持仓revision不改旧envelope，发送日期不冒充business_date。；R11持仓复盘：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r13 — R13观察池跟踪

owner：business_date_once_claims(business_date,WatchlistTracking,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,R13)。Epic：盘后；producer：review-r13-auto, review-r13-backfill, review-r13-manual, startup-resume-watchlist-tracking。快照SHA：`11589731da7bc5bf6282b46a42b22e8038ba7fd7326831363a722bfb2d3d039f`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/18h；PERT=10.67h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：观察池有效输入的盘后窗口；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：业务outcome与通知游标双写需CAS故障覆盖。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：WatchlistTracking原日期claim共享auto/manual/backfill；append audit后才commit task state，重试不再次推进watchlist outcome。；R13观察池跟踪：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-a10 — A10催化复盘

owner：business_date_once_claims(business_date,CatalystReview,None,GLOBAL) → immutable decision / occurrence=review_task_identity(date,A10)。Epic：盘后；producer：review-a10-auto, review-a10-backfill, review-a10-manual, review-a10-push, startup-resume-catalyst-review。快照SHA：`7b81df249f49b286dfb59444b26e429b5d8b73cc6c24db34eefc74036ec93c33`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=7/11/18h；PERT=11.5h；风险=high；外部等待=1工作日；owner change=true；rank=9；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：催化复盘eligible窗口及四入口回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：多一个显式push入口增加去重和审计分支。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：A10 auto/manual/backfill/--push四入口只在同immutable decision下hydrate；催化事件修订独立，新invocation audit不授权再次送达。；A10催化复盘：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

#### MU-review-r03-auto — R03自动任务STARVED

owner：post_session_review_scheduler::ReviewScheduleState(date).tasks&#91;R03&#93;。Epic：盘后；producer：review-r03-auto。快照SHA：`657e3873f935b219cd9ced22b179055c9ded8da2dab9de5d4e9ccf5e2c8d9cf3`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=4/8/14h；PERT=8.33h；风险=high；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：隔离账户合同缺口，来源恢复另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：F07固定缺口应ProducerUnready，不归咎用户快照或轮询。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED新claim未激活；LegacyAccountGate缺verified broker batch及同批trade-sync watermark可见，零消息不是通过；自动tasks[R03]只由核验旧decision hydrate。

#### MU-review-r03-manual — R03手动任务STARVED

owner：run_review_only::temporary audit_state(invocation,date).tasks&#91;R03&#93;。Epic：盘后；producer：review-r03-manual。快照SHA：`917f69da6d89a475f93e583667cfe9a545cfb6b535ea53e07405a9f61ee868a7`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=4/7/12h；PERT=7.33h；风险=high；外部等待=3工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W15, W16, W17, W18, W19, W20, W21。日历：仅manual拒绝/历史hydrate回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：F07手动临时状态独立于auto，不合并潜在durable owner。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED新claim未激活；账户批次/水位缺依赖可见，零消息不是通过；每invocation临时audit不能冒充auto task owner或创建新claim。

#### MU-paper-review-daily — 每日PaperReview STARVED

owner：COOLDOWN_TABLE(PaperReview,code)；L4(paper_review,code,空 sub_kind)。Epic：盘后；producer：paper-review-daily-auto, paper-review-daily-manual, paper-review-daily-push。快照SHA：`6a1bf7c5ecc9814cfa1412be8dfbb545854ddd0e30fe74ee4a4e057f33ee5034`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=5/9/16h；PERT=9.5h；风险=high；外部等待=2工作日；owner change=false；rank=10；晋级/观察=0/0 session。

依赖：W01, W02, W03, W04, W06, W07, W08, W09, W10, W11, W12, W14, W15, W16, W17, W18, W19, W20, W21。日历：历史T+1隔离样本，生产激活另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank10不能把daily/manual/--push误写成现成counted decision。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED新链未激活；A01自产链缺依赖可见，零消息不是通过；合法exact已完成T+1历史记录可conformance，排除自动backfill并保留实际code。

#### MU-block-confirm — 大宗交易逐条确认

owner：COOLDOWN_TABLE(BlockTradeIntradayConfirm,code)；L4(block_trade_intraday_confirm,code,空 sub_kind)。Epic：盘后；producer：block-confirm-side-route。快照SHA：`047156d8d035761b9ea2e45611c99e04d860de2135019e21dda300ce8585ab36`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/18h；PERT=10.67h；风险=high；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：交易记录自然出现及跨日回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：从两层冷却补真实逐条identity有设计风险。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：逐真实交易记录/历史business_date identity，300s模板/L4只冷却；同票两记录分别receipt，不虚构ReviewTask或批次日门。

#### MU-ipo-catalyst — IPO催化事件

owner：COOLDOWN_TABLE(IpoCatalyst,空 code)；L4(ipo_catalyst,空 code,空 sub_kind)。Epic：盘后；producer：ipo-catalyst-side-route。快照SHA：`8ce28f1c4086219cf88ee9d92a69486f51532a4cb151ee4ad17149a178f46759`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/9/15h；PERT=9.33h；风险=medium；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：公告/stage自然变化或灰盒；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：无日LAST/claim，需要独立通知cursor。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：date+announcement+stage形成occurrence；R08日期公告cache仅输入复用，空code两层冷却不得合并不同IPO阶段。

#### MU-cli-replay-force — 显式历史强制重放

owner：MonitorReplayPublisher replay envelope.id → replay_audit/YYYY.jsonl attempt/result hash chain；ReplayRunner invocation summary。Epic：盘中/盘前/盘后/集合竞价；producer：cli-replay-force。快照SHA：`9c967d9fd51e6d06b4da5155faee51070c189d643ebe2a0fde33cc8dd32f71ca`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=9/14/25h；PERT=15h；风险=critical；外部等待=1工作日；owner change=true；rank=null；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：操作员批准重放窗口，两eligible session；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：独立replay owner不是普通startup或原业务claim；本批仅测试合同不执行发送。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：真实发送审计绑定原消息hash与新replay identity及required-channel receipt；dry-run零发送，force显式授权才能新attempt，跨进程崩溃不能借本地hash链证明Accepted。

#### MU-cli-single — CLI单股报告

owner：AnalysisPipeline::process_stock_inner(invocation,code) 的 Option<AnalysisResult>；无持久通知 completion cursor。Epic：盘中/盘前/盘后/集合竞价；producer：cli-single-default, cli-single-lhb, cli-single-schedule。快照SHA：`207eb4da6553f038d1fb9d3fabebdb1cf33198fa389c80997cf4251be952f7e4`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=4/7/12h；PERT=7.33h；风险=medium；外部等待=0工作日；owner change=true；rank=1；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：CLI调用与schedule/LHB各一次隔离回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank1最小typed结果切口，跨调用无原持久cursor。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：default/schedule/LHB每invocation+code通知结果为typed BestEffort；Option AnalysisResult或业务DB成功不推断强receipt，空结果不造通知。

#### MU-cli-summary — CLI汇总报告

owner：AnalysisPipeline::run(invocation) 的 results / send_summary_notification_to 返回值；无持久通知 completion cursor。Epic：盘中/盘前/盘后/集合竞价；producer：cli-summary-default, cli-summary-lhb, cli-summary-schedule。快照SHA：`f30beea312d5bcd4c154c65cb5968b17c431ef39ecf1a0c74c1d04e369e1abde`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=5/8/13h；PERT=8.33h；风险=medium；外部等待=0工作日；owner change=true；rank=1；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：三个CLI入口非空结果集回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank1汇总形态与单股通知分开验证。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：nonempty results汇总保存独立通知结果；文件同分钟重名不当作completion，single成功不能完成summary。

#### MU-cli-chain — CLI产业链报告

owner：run_chain_analysis_mode(invocation) 的 Result<()>；无独立持久通知 cursor。Epic：盘中/盘前/盘后/集合竞价；producer：cli-chain。快照SHA：`f99e1b311f12062db0002c69d68074a5e4532493a98d71143ffeb5d42e9edccf`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=4/8/14h；PERT=8.33h；风险=medium；外部等待=0工作日；owner change=true；rank=null；晋级/观察=1/1 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W16, W17, W18, W19, W20, W21。日历：显式CLI invocation回放；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：CLI产业链报告的历史批准范围有歧义：蓝图明确default CLI单股/汇总，未明确纳入独立run_chain_analysis_mode owner。保持rank=null，纳入波次需要另行产品裁决；工程范围仍保留，且不合并R03/I03或两个timer。。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：Result<()>只代表函数完成，typed BestEffort逐渠道呈现；latest completed business date绑定payload，不借timer日期门授权。

#### MU-chain-preopen — 09:05产业链定时

owner：monitor_loop::CHAIN_PREOPEN_LAST&#91;calendar_date&#93;。Epic：盘前；producer：chain-preopen-timer。快照SHA：`0c1944dbae3ad3fa21f7f4851cebbe7dbcde5d59dae46cf8047395271ea8b9a9`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/10/16h；PERT=10.33h；风险=high；外部等待=1工作日；owner change=true；rank=2；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：交易日09:05至09:15自然occurrence；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank2 timer现无交易日guard，日期混淆需窗口回放。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：09:05≤t<09:15交易日guard；calendar封口日与latest completed业务日分开，跨自然日同业务结果不盲重发，CHAIN_PREOPEN_LAST按通知结果推进。

#### MU-chain-post-close — 15:30产业链定时

owner：monitor_loop::CHAIN_POST_LAST&#91;calendar_date&#93;。Epic：盘后；producer：chain-post-close-timer。快照SHA：`5596c2836c2023c75a26490bc5b5e1f3970e61b0b5eff025a22ac8f4fb9ff6f9`。

逐Unit接线、六门禁证据、cutover准备/核验及tail-cleanup资格核验；不含自然等待。 O/M/P=6/11/18h；PERT=11.33h；风险=high；外部等待=1工作日；owner change=true；rank=3；晋级/观察=1/2 session。

依赖：W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11, W12, W14, W16, W17, W18, W19, W20, W21。日历：交易日15:30至15:35自然occurrence；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：rank3五分钟窗口与分析耗时竞争，完成cursor独立。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：15:30≤t<15:35独立occurrence；CHAIN_POST_LAST不借preopen/CLI结果；同分钟报告文件覆盖不证明新通知成功。

#### MU-review-r03-stored-recovery — R03既存事实恢复

owner：business_date_once_claims(business_date,IndustryChain,None,GLOBAL) → existing immutable decision / occurrence=review_task_identity(date,R03)。Epic：盘后；producer：startup-resume-industry-chain。快照SHA：`f9850a92694dc1fcc215d432ea91aeed223685b6930572591d8a506b82f5b5d9`。

隔离conformance与保持未激活；不切换physical owner，不补生产依赖。 O/M/P=6/10/19h；PERT=10.83h；风险=high；外部等待=2工作日；owner change=false；rank=null；晋级/观察=0/0 session。

依赖：W01, W02, W03, W06, W07, W08, W09, W10, W11, W12, W13, W14, W15, W16, W17, W18, W19, W20, W21。日历：启动all-date旧信封隔离恢复，来源激活另批；人工批准与样本不足可无限延期；非交易日不消耗交易session。。估算依据：独立既存durable owner不能与auto/manual临时任务状态合并。

六类共享门禁：unit, failure, crash, shadow, dedup, rollback（每Unit/build重新取证）；专属门禁：保持STARVED新claim未激活；账户依赖缺失可见，零消息不是通过；只恢复原业务日existing IndustryChain decision，Rejected/Uncertain hydrate不能当已送达。；R03既存事实恢复：普通startup恢复旧事实及原immutable bytes/decision，不制造新occurrence或重复发送；Uncertain保留人工处置，恢复职责不授予新生产资格。

发布边界：draft校验只证明文档一致性；strict必须返回 wbs_status_provisional，不能用本WBS宣称实现、部署或真实接收完成。

<!-- RFC-WBS-END -->

## 规范 DDL 原始嵌入（PROPOSED）

独立 SQL 文件是唯一事实源；本节只复制原始字节，不维护手写变体。[Q:76] [Q:97]

SQL SHA-256：4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953

<!-- RFC-SQL-BEGIN -->
```sql
.bail on
-- SQLite CLI schema script；不能直接传给 library execute_batch。
-- 固定兼容签名是版本身份，不是 SQLite 计算的内容哈希。
-- 修订：v1-final-wave1-not-delivered；旧 v1 签名不兼容，拒绝自动迁移。
-- PROPOSED：仅用于新建临时数据库验证；不是生产迁移器。
-- 每个业务连接必须再次启用外键与递归 trigger；时间均为非负 UTC 微秒。
PRAGMA foreign_keys=ON;
PRAGMA recursive_triggers=ON;
BEGIN IMMEDIATE;

-- 在任何持久对象 CREATE 前冻结入口对象；临时审计行允许同一连接重复执行。
CREATE TEMP TABLE IF NOT EXISTS _push_v1_managed(name TEXT PRIMARY KEY, object_type TEXT NOT NULL);
INSERT INTO _push_v1_managed(name,object_type)
SELECT column1,column2 FROM (VALUES
  ('push_foundation_schema','table'),
  ('push_foundation_objects','table'),
  ('push_intents','table'),
  ('push_intents_recovery','index'),
  ('push_intents_insert_guard','trigger'),
  ('push_intents_immutable','trigger'),
  ('push_intents_delete','trigger'),
  ('push_intents_cas','trigger'),
  ('push_intent_transitions','table'),
  ('push_intent_transitions_binding','trigger'),
  ('push_intent_transitions_update','trigger'),
  ('push_intent_transitions_delete','trigger'),
  ('push_activation_manifests','table'),
  ('push_activation_manifests_chain','trigger'),
  ('push_activation_manifests_update','trigger'),
  ('push_activation_manifests_delete','trigger'),
  ('push_promotion_journal','table'),
  ('push_promotion_journal_binding','trigger'),
  ('push_promotion_journal_update','trigger'),
  ('push_promotion_journal_delete','trigger'),
  ('push_foundation_schema_update','trigger'),
  ('push_foundation_schema_delete','trigger'),
  ('push_foundation_objects_insert','trigger'),
  ('push_foundation_objects_update','trigger'),
  ('push_foundation_objects_delete','trigger')
) WHERE NOT EXISTS(SELECT 1 FROM _push_v1_managed m WHERE m.name=column1);
CREATE TEMP TABLE IF NOT EXISTS _push_v1_probe(id INTEGER PRIMARY KEY, phase TEXT, ok INTEGER);
CREATE TEMP TRIGGER IF NOT EXISTS _push_v1_guard BEFORE INSERT ON _push_v1_probe
WHEN NEW.phase='check' AND NEW.ok IS NOT 1
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
CREATE TEMP TABLE IF NOT EXISTS _push_v1_snapshot(run_id INTEGER, name TEXT, object_type TEXT, definition TEXT);
INSERT INTO _push_v1_probe(phase,ok)
SELECT 'probe',NOT EXISTS(SELECT 1 FROM sqlite_master WHERE sql IS NOT NULL AND (name IN (SELECT name FROM _push_v1_managed) OR tbl_name IN (SELECT name FROM _push_v1_managed WHERE object_type='table')));
INSERT INTO _push_v1_snapshot(run_id,name,object_type,definition)
SELECT (SELECT MAX(id) FROM _push_v1_probe WHERE phase='probe'),name,type,sql
FROM sqlite_master WHERE sql IS NOT NULL AND (name IN (SELECT name FROM _push_v1_managed) OR tbl_name IN (SELECT name FROM _push_v1_managed WHERE object_type='table'));


CREATE TABLE IF NOT EXISTS push_foundation_schema (
  version INTEGER PRIMARY KEY CHECK(version=1),
  description TEXT NOT NULL CHECK(description='push-foundation-v1'),
  schema_signature TEXT NOT NULL CHECK(typeof(schema_signature)='text' AND length(schema_signature)=64 AND length(CAST(schema_signature AS BLOB))=64 AND schema_signature NOT GLOB '*[^0-9a-f]*' AND schema_signature='dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd')
);


CREATE TABLE IF NOT EXISTS push_foundation_objects (
  name TEXT PRIMARY KEY NOT NULL,
  object_type TEXT NOT NULL CHECK(object_type IN ('table','index','trigger','view')),
  definition TEXT NOT NULL
);
-- 非全新库必须匹配入口快照，不能在补建对象后把缺失当兼容。
INSERT INTO _push_v1_probe(phase,ok)
SELECT 'check',
  (SELECT ok FROM _push_v1_probe WHERE phase='probe' ORDER BY id DESC LIMIT 1)=1
  OR (
    (SELECT count(*) FROM push_foundation_schema)=1
    AND EXISTS(SELECT 1 FROM push_foundation_schema WHERE version=1 AND description='push-foundation-v1' AND schema_signature='dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd')
    AND (SELECT count(*) FROM push_foundation_objects)=(SELECT count(*) FROM _push_v1_managed)
    AND NOT EXISTS(SELECT 1 FROM push_foundation_objects r WHERE NOT EXISTS(SELECT 1 FROM _push_v1_managed m WHERE m.name=r.name AND m.object_type=r.object_type))
    AND (SELECT count(*) FROM push_foundation_objects)=(SELECT count(*) FROM _push_v1_snapshot WHERE run_id=(SELECT MAX(id) FROM _push_v1_probe WHERE phase='probe'))
    AND NOT EXISTS(SELECT 1 FROM push_foundation_objects r WHERE NOT EXISTS(
      SELECT 1 FROM _push_v1_snapshot s WHERE s.run_id=(SELECT MAX(id) FROM _push_v1_probe WHERE phase='probe')
      AND s.name=r.name AND s.object_type=r.object_type AND CAST(s.definition AS BLOB)=CAST(r.definition AS BLOB)))
  );

CREATE TABLE IF NOT EXISTS push_intents (
  intent_id TEXT NOT NULL CHECK(typeof(intent_id)='text' AND length(intent_id)=64 AND length(CAST(intent_id AS BLOB))=64 AND intent_id NOT GLOB '*[^0-9a-f]*') PRIMARY KEY,
  job_decision_kind TEXT NOT NULL CHECK(job_decision_kind IN ('Ready','NoData','Disabled')),
  namespace TEXT NOT NULL CHECK(length(namespace) BETWEEN 1 AND 512),
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  occurrence_family TEXT NOT NULL CHECK(length(occurrence_family) BETWEEN 1 AND 512),
  occurrence_key TEXT NOT NULL CHECK(length(occurrence_key) BETWEEN 1 AND 512),
  completion_owner TEXT NOT NULL CHECK(length(completion_owner) BETWEEN 1 AND 512),
  source_contract_id TEXT NOT NULL CHECK(length(source_contract_id) BETWEEN 1 AND 512),
  subject TEXT NOT NULL CHECK(length(subject) BETWEEN 1 AND 512),
  audience TEXT NOT NULL CHECK(length(audience) BETWEEN 1 AND 512),
  durable_decision_id TEXT NOT NULL CHECK(length(durable_decision_id) BETWEEN 1 AND 512),
  business_date TEXT NOT NULL CHECK(length(business_date)=10 AND business_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]' AND date(business_date,'+0 days') IS business_date),
  prepared_push_bytes BLOB CHECK(prepared_push_bytes IS NULL OR (typeof(prepared_push_bytes)='blob' AND length(prepared_push_bytes)>0)),
  rendered_bytes BLOB CHECK(rendered_bytes IS NULL OR (typeof(rendered_bytes)='blob' AND length(rendered_bytes)>0)),
  payload_sha256 TEXT CHECK(payload_sha256 IS NULL OR (typeof(payload_sha256)='text' AND length(payload_sha256)=64 AND length(CAST(payload_sha256 AS BLOB))=64 AND payload_sha256 NOT GLOB '*[^0-9a-f]*')),
  rendered_sha256 TEXT CHECK(rendered_sha256 IS NULL OR (typeof(rendered_sha256)='text' AND length(rendered_sha256)=64 AND length(CAST(rendered_sha256 AS BLOB))=64 AND rendered_sha256 NOT GLOB '*[^0-9a-f]*')),
  evidence_sha256 TEXT NOT NULL CHECK(typeof(evidence_sha256)='text' AND length(evidence_sha256)=64 AND length(CAST(evidence_sha256 AS BLOB))=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  template_sha256 TEXT NOT NULL CHECK(typeof(template_sha256)='text' AND length(template_sha256)=64 AND length(CAST(template_sha256 AS BLOB))=64 AND template_sha256 NOT GLOB '*[^0-9a-f]*'),
  source_contract_sha256 TEXT NOT NULL CHECK(typeof(source_contract_sha256)='text' AND length(source_contract_sha256)=64 AND length(CAST(source_contract_sha256 AS BLOB))=64 AND source_contract_sha256 NOT GLOB '*[^0-9a-f]*'),
  state TEXT NOT NULL CHECK(state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NotDelivered','NoData','Disabled','ResolutionRequired')),
  previous_state TEXT CHECK(previous_state IS NULL OR previous_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NotDelivered','NoData','Disabled','ResolutionRequired')),
  reason TEXT NOT NULL CHECK(typeof(reason)='text' AND length(CAST(reason AS BLOB))=length(reason) AND length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  lease_owner TEXT CHECK(lease_owner IS NULL OR length(lease_owner) BETWEEN 1 AND 512),
  lease_until INTEGER CHECK(lease_until IS NULL OR (typeof(lease_until)='integer' AND lease_until>=0)),
  lease_generation INTEGER NOT NULL DEFAULT 0 CHECK(typeof(lease_generation)='integer' AND lease_generation>=0),
  version INTEGER NOT NULL DEFAULT 0 CHECK(typeof(version)='integer' AND version>=0),
  created_at INTEGER NOT NULL CHECK(typeof(created_at)='integer' AND created_at>=0),
  updated_at INTEGER NOT NULL CHECK(typeof(updated_at)='integer' AND updated_at>=0),
  CHECK(updated_at>=created_at),
  CHECK((job_decision_kind='Ready' AND prepared_push_bytes IS NOT NULL AND rendered_bytes IS NOT NULL AND payload_sha256 IS NOT NULL AND rendered_sha256 IS NOT NULL)
    OR (job_decision_kind IN ('NoData','Disabled') AND prepared_push_bytes IS NULL AND rendered_bytes IS NULL AND payload_sha256 IS NULL AND rendered_sha256 IS NULL)),
  CHECK(job_decision_kind='Ready' OR state IN ('NoData','Disabled','ResolutionRequired')),
  CHECK((lease_owner IS NULL)=(lease_until IS NULL)),
  UNIQUE(namespace,unit_id,completion_owner,source_contract_id,business_date,occurrence_family,occurrence_key,subject,audience),
  UNIQUE(namespace,durable_decision_id)
);
CREATE INDEX IF NOT EXISTS push_intents_recovery ON push_intents(state,lease_until,unit_id);
CREATE TRIGGER IF NOT EXISTS push_intents_insert_guard
BEFORE INSERT ON push_intents
WHEN EXISTS(SELECT 1 FROM push_intents WHERE intent_id=NEW.intent_id)
  OR NEW.version<>0 OR NEW.previous_state IS NOT NULL OR NEW.lease_generation<>0 OR NEW.lease_owner IS NOT NULL
  OR NOT ((NEW.job_decision_kind='Ready' AND NEW.state='PendingDispatch' AND NEW.reason='intent.created')
    OR (NEW.job_decision_kind='NoData' AND NEW.state='NoData' AND NEW.reason='intent.no_data')
    OR (NEW.job_decision_kind='Disabled' AND NEW.state='Disabled' AND NEW.reason='policy.disabled'))
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.insert_conflict');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_immutable
BEFORE UPDATE ON push_intents
WHEN NEW.intent_id IS NOT OLD.intent_id OR
  NEW.job_decision_kind IS NOT OLD.job_decision_kind OR
  NEW.namespace IS NOT OLD.namespace OR
  NEW.unit_id IS NOT OLD.unit_id OR
  NEW.occurrence_family IS NOT OLD.occurrence_family OR
  NEW.occurrence_key IS NOT OLD.occurrence_key OR
  NEW.completion_owner IS NOT OLD.completion_owner OR
  NEW.source_contract_id IS NOT OLD.source_contract_id OR
  NEW.subject IS NOT OLD.subject OR
  NEW.audience IS NOT OLD.audience OR
  NEW.durable_decision_id IS NOT OLD.durable_decision_id OR
  NEW.business_date IS NOT OLD.business_date OR
  NEW.created_at IS NOT OLD.created_at OR
  NEW.prepared_push_bytes IS NOT OLD.prepared_push_bytes OR
  NEW.rendered_bytes IS NOT OLD.rendered_bytes OR
  NEW.payload_sha256 IS NOT OLD.payload_sha256 OR
  NEW.rendered_sha256 IS NOT OLD.rendered_sha256 OR
  NEW.evidence_sha256 IS NOT OLD.evidence_sha256 OR
  NEW.template_sha256 IS NOT OLD.template_sha256 OR
  NEW.source_contract_sha256 IS NOT OLD.source_contract_sha256
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.immutable');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_delete
BEFORE DELETE ON push_intents
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.delete_forbidden');
END;
CREATE TRIGGER IF NOT EXISTS push_intents_cas
BEFORE UPDATE ON push_intents
WHEN NEW.version<>OLD.version+1 OR NEW.previous_state IS NOT OLD.state OR NEW.updated_at<OLD.updated_at
  OR NEW.lease_generation<OLD.lease_generation OR NEW.lease_generation>OLD.lease_generation+1
  OR (NEW.lease_owner IS NOT OLD.lease_owner AND NEW.lease_owner IS NOT NULL AND NEW.lease_generation<>OLD.lease_generation+1)
  OR (NEW.lease_owner IS NOT OLD.lease_owner AND NEW.lease_owner IS NOT NULL AND OLD.lease_owner IS NOT NULL AND NEW.updated_at<OLD.lease_until)
  OR NOT (
    (NEW.state=OLD.state AND OLD.state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','ResolutionRequired') AND NEW.reason IN ('intent.lease_held','intent.dispatch_claimed'))
    OR (NEW.state=OLD.state AND OLD.state='AwaitingAuthority' AND NEW.reason IN ('transport.rejected','finalizer.terminal_ref_invalid'))
    OR (NEW.state=OLD.state AND OLD.state='AwaitingFinalizer' AND NEW.reason='finalizer.terminal_ref_invalid')
    OR (OLD.state='PendingDispatch' AND NEW.state='AwaitingAuthority' AND NEW.reason='intent.dispatch_claimed')
    OR (OLD.state='PendingDispatch' AND NEW.state='NoData' AND NEW.reason='intent.no_data')
    OR (OLD.state='PendingDispatch' AND NEW.state='Disabled' AND NEW.reason='policy.disabled')
    OR (OLD.state IN ('AwaitingAuthority','ResolutionRequired') AND NEW.job_decision_kind='Ready' AND NEW.state='AwaitingFinalizer' AND NEW.reason='intent.authority_verified')
    OR (OLD.state='AwaitingFinalizer' AND NEW.state='Completed' AND NEW.reason='finalizer.completed')
    OR (NEW.state='NotDelivered' AND NEW.job_decision_kind='Ready' AND NEW.reason='operator.not_delivered'
      AND NOT EXISTS(SELECT 1 FROM push_intent_transitions a WHERE a.intent_id=OLD.intent_id AND a.to_state IN ('AwaitingFinalizer','Completed'))
      AND (OLD.state='AwaitingAuthority' OR (OLD.state='ResolutionRequired' AND EXISTS(
        SELECT 1 FROM push_intent_transitions u WHERE u.intent_id=OLD.intent_id
          AND u.to_state='ResolutionRequired' AND u.from_state='AwaitingAuthority' AND u.reason='transport.uncertain'
          AND u.result_version=(SELECT MAX(r.result_version) FROM push_intent_transitions r
            WHERE r.intent_id=OLD.intent_id AND r.to_state='ResolutionRequired' AND r.from_state<>'ResolutionRequired')))))

    OR (OLD.state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NoData','Disabled') AND NEW.state='ResolutionRequired' AND NEW.reason IN ('intent.payload_conflict','intent.expected_version_conflict','finalizer.cas_conflict'))
    OR (OLD.state IN ('AwaitingAuthority','AwaitingFinalizer') AND NEW.state='ResolutionRequired' AND NEW.reason IN ('transport.uncertain','operator.resolution_conflict'))
  )
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.cas_or_edge_invalid');
END;

CREATE TABLE IF NOT EXISTS push_intent_transitions (
  event_id TEXT NOT NULL CHECK(typeof(event_id)='text' AND length(event_id)=64 AND length(CAST(event_id AS BLOB))=64 AND event_id NOT GLOB '*[^0-9a-f]*') PRIMARY KEY,
  intent_id TEXT NOT NULL CHECK(typeof(intent_id)='text' AND length(intent_id)=64 AND length(CAST(intent_id AS BLOB))=64 AND intent_id NOT GLOB '*[^0-9a-f]*') REFERENCES push_intents(intent_id),
  from_state TEXT NOT NULL CHECK(from_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NotDelivered','NoData','Disabled','ResolutionRequired')),
  to_state TEXT NOT NULL CHECK(to_state IN ('PendingDispatch','AwaitingAuthority','AwaitingFinalizer','Completed','NotDelivered','NoData','Disabled','ResolutionRequired')),
  expected_version INTEGER NOT NULL CHECK(typeof(expected_version)='integer' AND expected_version>=0),
  result_version INTEGER NOT NULL CHECK(typeof(result_version)='integer' AND result_version=expected_version+1),
  previous_sha256 TEXT CHECK(previous_sha256 IS NULL OR (typeof(previous_sha256)='text' AND length(previous_sha256)=64 AND length(CAST(previous_sha256 AS BLOB))=64 AND previous_sha256 NOT GLOB '*[^0-9a-f]*')),
  canonical_sha256 TEXT NOT NULL CHECK(typeof(canonical_sha256)='text' AND length(canonical_sha256)=64 AND length(CAST(canonical_sha256 AS BLOB))=64 AND canonical_sha256 NOT GLOB '*[^0-9a-f]*'),
  actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 512),
  reason TEXT NOT NULL CHECK(typeof(reason)='text' AND length(CAST(reason AS BLOB))=length(reason) AND length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  terminal_disposition TEXT CHECK(terminal_disposition IS NULL OR terminal_disposition IN ('Accepted','ManualConfirmedAccepted','ManualConfirmedNotDelivered')),
  terminal_decision_id TEXT CHECK(terminal_decision_id IS NULL OR length(terminal_decision_id) BETWEEN 1 AND 512),
  operator_audit_ref TEXT CHECK(operator_audit_ref IS NULL OR length(operator_audit_ref) BETWEEN 1 AND 512),
  operator_audit_sha256 TEXT CHECK(operator_audit_sha256 IS NULL OR (typeof(operator_audit_sha256)='text' AND length(operator_audit_sha256)=64 AND length(CAST(operator_audit_sha256 AS BLOB))=64 AND operator_audit_sha256 NOT GLOB '*[^0-9a-f]*')),
  terminal_ref_id TEXT CHECK(terminal_ref_id IS NULL OR length(terminal_ref_id) BETWEEN 1 AND 512),
  terminal_binding_sha256 TEXT CHECK(terminal_binding_sha256 IS NULL OR (typeof(terminal_binding_sha256)='text' AND length(terminal_binding_sha256)=64 AND length(CAST(terminal_binding_sha256 AS BLOB))=64 AND terminal_binding_sha256 NOT GLOB '*[^0-9a-f]*')),
  occurred_at INTEGER NOT NULL CHECK(typeof(occurred_at)='integer' AND occurred_at>=0),
  CHECK((terminal_ref_id IS NULL)=(terminal_binding_sha256 IS NULL)),
  CHECK((to_state IN ('Completed','NotDelivered'))=(terminal_ref_id IS NOT NULL)),
  CHECK((to_state IN ('Completed','NotDelivered'))=(terminal_disposition IS NOT NULL)),
  CHECK(to_state<>'Completed' OR terminal_disposition IN ('Accepted','ManualConfirmedAccepted')),
  CHECK((to_state='NotDelivered')=(terminal_decision_id IS NOT NULL)),
  CHECK((to_state='NotDelivered')=(operator_audit_ref IS NOT NULL)),
  CHECK((to_state='NotDelivered')=(operator_audit_sha256 IS NOT NULL)),
  CHECK(to_state<>'NotDelivered' OR (terminal_disposition='ManualConfirmedNotDelivered' AND reason='operator.not_delivered')),
  CHECK((result_version=1)=(previous_sha256 IS NULL)),
  UNIQUE(intent_id,result_version)
);
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_binding
BEFORE INSERT ON push_intent_transitions
WHEN EXISTS(SELECT 1 FROM push_intent_transitions WHERE event_id=NEW.event_id)
  OR NOT EXISTS(SELECT 1 FROM push_intents i WHERE i.intent_id=NEW.intent_id AND i.state=NEW.to_state AND i.previous_state=NEW.from_state AND i.version=NEW.result_version AND i.reason=NEW.reason AND i.updated_at<=NEW.occurred_at)
  OR (NEW.to_state='NotDelivered' AND NOT EXISTS(SELECT 1 FROM push_intents i
    WHERE i.intent_id=NEW.intent_id AND i.durable_decision_id=NEW.terminal_decision_id))
  OR (NEW.result_version>1 AND NOT EXISTS(SELECT 1 FROM push_intent_transitions p WHERE p.intent_id=NEW.intent_id AND p.result_version=NEW.expected_version AND p.to_state=NEW.from_state AND p.canonical_sha256=NEW.previous_sha256))
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.transition_binding_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_update
BEFORE UPDATE ON push_intent_transitions
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_intent_transitions_delete
BEFORE DELETE ON push_intent_transitions
BEGIN
  SELECT RAISE(ROLLBACK, 'intent.append_only');
END;

CREATE TABLE IF NOT EXISTS push_activation_manifests (
  manifest_sha256 TEXT NOT NULL CHECK(typeof(manifest_sha256)='text' AND length(manifest_sha256)=64 AND length(CAST(manifest_sha256 AS BLOB))=64 AND manifest_sha256 NOT GLOB '*[^0-9a-f]*') PRIMARY KEY,
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=1),
  previous_manifest_sha256 TEXT CHECK(previous_manifest_sha256 IS NULL OR (typeof(previous_manifest_sha256)='text' AND length(previous_manifest_sha256)=64 AND length(CAST(previous_manifest_sha256 AS BLOB))=64 AND previous_manifest_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  desired_state TEXT NOT NULL CHECK(desired_state IN ('Disabled','Shadow','Active','Draining')),
  physical_owner TEXT NOT NULL CHECK(length(physical_owner) BETWEEN 1 AND 512),
  build_commit TEXT NOT NULL CHECK(typeof(build_commit)='text' AND length(build_commit)=40 AND length(CAST(build_commit AS BLOB))=40 AND build_commit NOT GLOB '*[^0-9a-f]*'),
  build_sha256 TEXT NOT NULL CHECK(typeof(build_sha256)='text' AND length(build_sha256)=64 AND length(CAST(build_sha256 AS BLOB))=64 AND build_sha256 NOT GLOB '*[^0-9a-f]*'),
  catalog_sha256 TEXT NOT NULL CHECK(typeof(catalog_sha256)='text' AND length(catalog_sha256)=64 AND length(CAST(catalog_sha256 AS BLOB))=64 AND catalog_sha256 NOT GLOB '*[^0-9a-f]*'),
  business_schema_sha256 TEXT NOT NULL CHECK(typeof(business_schema_sha256)='text' AND length(business_schema_sha256)=64 AND length(CAST(business_schema_sha256 AS BLOB))=64 AND business_schema_sha256 NOT GLOB '*[^0-9a-f]*'),
  durable_schema_sha256 TEXT NOT NULL CHECK(typeof(durable_schema_sha256)='text' AND length(durable_schema_sha256)=64 AND length(CAST(durable_schema_sha256 AS BLOB))=64 AND durable_schema_sha256 NOT GLOB '*[^0-9a-f]*'),
  template_sha256 TEXT NOT NULL CHECK(typeof(template_sha256)='text' AND length(template_sha256)=64 AND length(CAST(template_sha256 AS BLOB))=64 AND template_sha256 NOT GLOB '*[^0-9a-f]*'),
  source_contract_sha256 TEXT NOT NULL CHECK(typeof(source_contract_sha256)='text' AND length(source_contract_sha256)=64 AND length(CAST(source_contract_sha256 AS BLOB))=64 AND source_contract_sha256 NOT GLOB '*[^0-9a-f]*'),
  evidence_sha256 TEXT NOT NULL CHECK(typeof(evidence_sha256)='text' AND length(evidence_sha256)=64 AND length(CAST(evidence_sha256 AS BLOB))=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  approved_by TEXT NOT NULL CHECK(length(approved_by) BETWEEN 1 AND 512),
  approved_at INTEGER NOT NULL CHECK(typeof(approved_at)='integer' AND approved_at>=0),
  window_start INTEGER NOT NULL CHECK(typeof(window_start)='integer' AND window_start>=0),
  window_end INTEGER NOT NULL CHECK(typeof(window_end)='integer' AND window_end>=0),
  rollback_target_sha256 TEXT CHECK(rollback_target_sha256 IS NULL OR (typeof(rollback_target_sha256)='text' AND length(rollback_target_sha256)=64 AND length(CAST(rollback_target_sha256 AS BLOB))=64 AND rollback_target_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  created_at INTEGER NOT NULL CHECK(typeof(created_at)='integer' AND created_at>=0),
  CHECK(window_end>window_start AND approved_at<=created_at),
  CHECK((generation=1)=(previous_manifest_sha256 IS NULL)),
  UNIQUE(unit_id,generation)
);
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_chain
BEFORE INSERT ON push_activation_manifests
WHEN EXISTS(SELECT 1 FROM push_activation_manifests WHERE manifest_sha256=NEW.manifest_sha256)
  OR NEW.generation<>COALESCE((SELECT MAX(generation)+1 FROM push_activation_manifests WHERE unit_id=NEW.unit_id),1)
  OR (NEW.generation=1 AND (NEW.desired_state<>'Disabled' OR NEW.rollback_target_sha256 IS NOT NULL))
  OR (NEW.generation>1 AND NOT EXISTS(SELECT 1 FROM push_activation_manifests p WHERE p.unit_id=NEW.unit_id AND p.generation=NEW.generation-1 AND p.manifest_sha256=NEW.previous_manifest_sha256))
  OR (NEW.generation>1 AND NEW.rollback_target_sha256 IS NULL AND NOT EXISTS(
    SELECT 1 FROM push_activation_manifests p WHERE p.manifest_sha256=NEW.previous_manifest_sha256 AND (
      (p.desired_state='Disabled' AND NEW.desired_state='Shadow') OR (p.desired_state='Shadow' AND NEW.desired_state='Active')
      OR (p.desired_state='Active' AND NEW.desired_state='Draining') OR (p.desired_state='Draining' AND NEW.desired_state='Disabled'))))
  OR (NEW.rollback_target_sha256 IS NOT NULL AND NOT EXISTS(SELECT 1 FROM push_activation_manifests r WHERE r.manifest_sha256=NEW.rollback_target_sha256 AND r.unit_id=NEW.unit_id AND r.generation<NEW.generation AND r.desired_state=NEW.desired_state AND r.physical_owner=NEW.physical_owner))
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.generation_or_edge_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_update
BEFORE UPDATE ON push_activation_manifests
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.history_immutable');
END;
CREATE TRIGGER IF NOT EXISTS push_activation_manifests_delete
BEFORE DELETE ON push_activation_manifests
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.history_immutable');
END;

CREATE TABLE IF NOT EXISTS push_promotion_journal (
  event_id TEXT NOT NULL CHECK(typeof(event_id)='text' AND length(event_id)=64 AND length(CAST(event_id AS BLOB))=64 AND event_id NOT GLOB '*[^0-9a-f]*') PRIMARY KEY,
  unit_id TEXT NOT NULL CHECK(length(unit_id) BETWEEN 1 AND 512),
  generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=1),
  from_manifest_sha256 TEXT CHECK(from_manifest_sha256 IS NULL OR (typeof(from_manifest_sha256)='text' AND length(from_manifest_sha256)=64 AND length(CAST(from_manifest_sha256 AS BLOB))=64 AND from_manifest_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  to_manifest_sha256 TEXT NOT NULL CHECK(typeof(to_manifest_sha256)='text' AND length(to_manifest_sha256)=64 AND length(CAST(to_manifest_sha256 AS BLOB))=64 AND to_manifest_sha256 NOT GLOB '*[^0-9a-f]*') REFERENCES push_activation_manifests(manifest_sha256),
  actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 512),
  action TEXT NOT NULL CHECK(action IN ('Initialize','EnterShadow','Activate','Drain','Disable','Rollback')),
  reason TEXT NOT NULL CHECK(typeof(reason)='text' AND length(CAST(reason AS BLOB))=length(reason) AND length(reason) BETWEEN 3 AND 96 AND reason NOT GLOB '*[^a-z0-9_.]*' AND substr(reason,1,instr(reason,'.')-1) IN ('schedule','input','policy','intent','transport','finalizer','activation','shadow','operator') AND substr(reason,instr(reason,'.')+1) GLOB '[a-z]*' AND instr(substr(reason,instr(reason,'.')+1),'.')=0),
  window_start INTEGER NOT NULL CHECK(typeof(window_start)='integer' AND window_start>=0),
  window_end INTEGER NOT NULL CHECK(typeof(window_end)='integer' AND window_end>=0),
  evidence_sha256 TEXT NOT NULL CHECK(typeof(evidence_sha256)='text' AND length(evidence_sha256)=64 AND length(CAST(evidence_sha256 AS BLOB))=64 AND evidence_sha256 NOT GLOB '*[^0-9a-f]*'),
  rollback_target_sha256 TEXT CHECK(rollback_target_sha256 IS NULL OR (typeof(rollback_target_sha256)='text' AND length(rollback_target_sha256)=64 AND length(CAST(rollback_target_sha256 AS BLOB))=64 AND rollback_target_sha256 NOT GLOB '*[^0-9a-f]*')) REFERENCES push_activation_manifests(manifest_sha256),
  previous_sha256 TEXT CHECK(previous_sha256 IS NULL OR (typeof(previous_sha256)='text' AND length(previous_sha256)=64 AND length(CAST(previous_sha256 AS BLOB))=64 AND previous_sha256 NOT GLOB '*[^0-9a-f]*')),
  canonical_sha256 TEXT NOT NULL CHECK(typeof(canonical_sha256)='text' AND length(canonical_sha256)=64 AND length(CAST(canonical_sha256 AS BLOB))=64 AND canonical_sha256 NOT GLOB '*[^0-9a-f]*'),
  occurred_at INTEGER NOT NULL CHECK(typeof(occurred_at)='integer' AND occurred_at>=0),
  CHECK(window_end>window_start AND occurred_at>=window_start AND occurred_at<window_end),
  CHECK((generation=1)=(from_manifest_sha256 IS NULL)),
  CHECK((generation=1)=(previous_sha256 IS NULL)),
  CHECK(reason='activation.applied'),
  CHECK((action='Rollback')=(rollback_target_sha256 IS NOT NULL)),
  UNIQUE(unit_id,generation)
);
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_binding
BEFORE INSERT ON push_promotion_journal
WHEN EXISTS(SELECT 1 FROM push_promotion_journal WHERE event_id=NEW.event_id)
  OR NEW.generation<>COALESCE((SELECT MAX(generation)+1 FROM push_promotion_journal WHERE unit_id=NEW.unit_id),1)
  OR NOT EXISTS(SELECT 1 FROM push_activation_manifests m WHERE m.manifest_sha256=NEW.to_manifest_sha256 AND m.unit_id=NEW.unit_id AND m.generation=NEW.generation AND m.previous_manifest_sha256 IS NEW.from_manifest_sha256 AND m.rollback_target_sha256 IS NEW.rollback_target_sha256 AND m.window_start=NEW.window_start AND m.window_end=NEW.window_end AND m.evidence_sha256=NEW.evidence_sha256 AND m.approved_by=NEW.actor AND m.approved_at<=NEW.occurred_at AND (
    (NEW.action='Initialize' AND m.generation=1 AND m.desired_state='Disabled')
    OR (NEW.action='EnterShadow' AND m.generation>1 AND m.desired_state='Shadow' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Activate' AND m.desired_state='Active' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Drain' AND m.desired_state='Draining' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Disable' AND m.generation>1 AND m.desired_state='Disabled' AND m.rollback_target_sha256 IS NULL)
    OR (NEW.action='Rollback' AND m.rollback_target_sha256 IS NOT NULL)))
  OR (NEW.generation>1 AND NOT EXISTS(SELECT 1 FROM push_promotion_journal p WHERE p.unit_id=NEW.unit_id AND p.generation=NEW.generation-1 AND p.to_manifest_sha256=NEW.from_manifest_sha256 AND p.canonical_sha256=NEW.previous_sha256))
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.journal_binding_invalid');
END;
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_update
BEFORE UPDATE ON push_promotion_journal
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_promotion_journal_delete
BEFORE DELETE ON push_promotion_journal
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.append_only');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_schema_update
BEFORE UPDATE ON push_foundation_schema
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_schema_delete
BEFORE DELETE ON push_foundation_schema
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;


CREATE TRIGGER IF NOT EXISTS push_foundation_objects_insert BEFORE INSERT ON push_foundation_objects
WHEN EXISTS(SELECT 1 FROM push_foundation_schema)
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_objects_update BEFORE UPDATE ON push_foundation_objects
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
CREATE TRIGGER IF NOT EXISTS push_foundation_objects_delete BEFORE DELETE ON push_foundation_objects
BEGIN
  SELECT RAISE(ROLLBACK, 'activation.manifest_mismatch');
END;
-- 仅首次空库登记；metadata 的定义与保护 trigger 本身也被登记。
INSERT INTO push_foundation_objects(name,object_type,definition)
SELECT name,type,sql FROM sqlite_master
WHERE sql IS NOT NULL AND name IN (SELECT name FROM _push_v1_managed)
  AND NOT EXISTS(SELECT 1 FROM push_foundation_schema);
INSERT INTO _push_v1_probe(phase,ok)
SELECT 'check',(SELECT count(*) FROM push_foundation_objects)=(SELECT count(*) FROM _push_v1_managed)
  AND NOT EXISTS(SELECT 1 FROM push_foundation_objects r WHERE NOT EXISTS(SELECT 1 FROM _push_v1_managed m WHERE m.name=r.name AND m.object_type=r.object_type));
INSERT INTO push_foundation_schema(version,description,schema_signature)
SELECT 1,'push-foundation-v1','dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd'
WHERE NOT EXISTS(SELECT 1 FROM push_foundation_schema);

COMMIT;
```
<!-- RFC-SQL-END -->
