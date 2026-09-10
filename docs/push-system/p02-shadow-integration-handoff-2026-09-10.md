# P-02 真实业务影子接线：当前证据与实施前置

日期：2026-09-10。依据提交 `8dbd3efe473e13a8fc31d4f4dbfeca71618a4873` 中未改的下列源码再次核对。本记录承接已完成的来源观察、冻结业务提案和 W17 内核，避免下一任务重复实现它们；不是新的影子 adapter 已交付、不是运行期或生产验收。当前唯一代码实现任务是 W18 请求入口，本文不启动第二个 Rust 写入者或 Cargo。

## 已有能力与真正缺口

| 位置 | 当前已实现 | 下一步必须补齐 |
| --- | --- | --- |
| [目录 Unit](push-capability-catalog.v1.json#L8813) | P-02 对应 `MU-auction-volume`，producer 为 `auction-volume` | 不把测试示例 `MU-p02` 等任意文本当正式注册；该目录是冻结历史，旧的“双次加载”说明不代表当前实现 |
| [观察对象](../../src/market_analyzer/limit_up.rs#L67)、[真实采集入口](../../src/market_analyzer/limit_up.rs#L497) | 私有字段保留涨停池、名称分片、原请求 hash、实际审计回执及原始股票 | 将同一次观察绑定到一次准备事实，不能重新采集以补证据；本地回执不是生产来源身份认证 |
| [真实 loader](../../src/bin/monitor/push_templates.rs#L6059)、[实际 main 消费](../../src/bin/monitor/main.rs#L9710) | loader 从观察的股票生成快照，main 保留完整 tick，P-02 与后续持仓检测借用它 | 新路径必须接在这个实际调用点，不能只增加无人调用的模拟入口 |
| [横幅捕获](../../src/bin/monitor/push_templates.rs#L256) | `CapturedBanner` 已冻结展示文本；捕获会读外部账户/估值说明 | 把捕获放在 old/new 纯投影之前，两侧只用同一结果；不能在影子 callback 内再次 `capture/render` 触发外部读取 |
| [完整业务提案](../../src/bin/monitor/push_templates.rs#L6081)、[判等](../../src/bin/monitor/push_templates.rs#L6112) | 同时包含 message、有序逐票 records、notified_codes；价格按 `to_bits` 比较，records 字段及顺序均参与 | 影子比较必须覆盖这三部分，而不只比较可见文案或文案 hash |
| [当前 dispatcher](../../src/bin/monitor/push_templates.rs#L6189) | 先冻结一次提案；sink 成功且所有 recorder 成功后才推进通知集合 | 引入比较后，发送/记录/集合推进仍消费被比较的同一旧提案；禁止比较完重新 prepare 一份 |
| [W17 执行入口](../../src/monitor/push_job/shadow.rs#L279)、[Ready 绑定校验](../../src/monitor/push_job/shadow.rs#L348) | 同 context/Arc facts，真实 JobDecision/语义/字节/完成提案比较及八类拒绝能力 | 还没有 P-02 实际 old/new adapter，也未将其完整业务提案纳入本次执行的结构化比较 |
| [PreparedPush](../../src/monitor/push_job/projection.rs#L713) | 绑定 intent/decision/context/facts/semantic/rendered bytes | 该对象没有 P-02 的逐票写库记录和通知集合，不能仅因其相等而断言完整 P-02 行为相等 |
| [上下文工厂](../../src/monitor/push_job/context.rs#L189)、[投影构造](../../src/monitor/push_job/projection.rs#L277) | 非 test 的 factory/type/构造逻辑已经存在，但 binding/input 字段和有效构造路径受限 | 缺的是可信注册与实际运行输入进入这些构造路径的 interface，不是“所有类型只在 cfg(test) 存在” |
| [机器注册字段](../../src/monitor/push_job/catalog.rs#L191) | Unit、completion owner、producer、occurrence family、phase 已有权威目录关系 | source contract/version、模板、audience、completion policy 不能从测试常量推定为生产注册；需明确实际注册来源和绑定 |

## 不能只比消息：已有独立反例

[完整提案测试](../../src/bin/monitor/push_templates.rs#L18030)逐字断言文案、两条 records 的全部业务字段和通知集合；[价格/指标/集合差异反例](../../src/bin/monitor/push_templates.rs#L18088)明确证明，价格改变或隐藏在显示舍入后的指标变化，可以让消息完全相同而业务记录不同。下一任务须复用这些独立期望并从实际影子 interface 检测差异，不能改成“同一个 prepare 调用两次，所以结果相等”。

业务提案的精确比较应与同次 W17 执行绑定，并保留旧提案供实际发送。可复用现有 `PartialEq`，但单独在测试比较两个提案、或由 caller 自报一个 payload hash，都不能证明运行时比较了完整实际输出。接口还须确保回调失败、未执行、任一拒绝端口非零时不能生成 Match；Debug 仅输出类型、数量和差异类别，不输出提案正文。

## 实施依赖顺序

1. **完成真实注册/捕获 interface 的具体设计。** 核对正式 Unit/producer/occurrence/owner 关系，将 source、模板、audience、completion policy 与部署/来源证据逐项绑定。不得放宽私有字段或增加任意“成功构造器”来让跨 crate 测试通过。这里不要求先完成全部 W15，但来源/部署声明与真正认证必须分清；缺实际配置的生产权限不可由本地默认值补齐。
2. **一次捕获，两个真实纯 adapter。** 同一个 RunContext、共享 PreparedFacts、同一来源观察、通知集合起始快照与 CapturedBanner 进入旧/新路径。两侧使用实际业务逻辑及 W05 投影，比较完整决策和业务提案；不能将 legacy 的一个结果克隆两份冒充两条执行，也不能二次读取行情、账户或墙上时钟。
3. **接实际消费与拒绝能力。** 比较结果绑定本次输入/Unit/输出，现有 dispatcher 消费同一旧提案。八类效果在影子路径实际接拒绝能力，计数先于拒绝；纯回调不能沙箱化任意全局 I/O，未纳管的旧全局调用不得获得“零调用”证明。
4. **接 W16 当前激活证据和共同 fence。** 影子 actor 无效果权限，Match 也不授予发送或晋级。初始 Disabled/Shadow 下旧 owner 是否保留、排空后的关闭及 rollback 新代恢复，严格沿[W16 已澄清合同](activation-contract-decisions-2026-09-08.md)，不能把整个 Unit 一律关掉，也不能凭影子失败或成功自行改 owner。
5. **形成逐 Unit 验收，再进入灰度。** 合成 Ready 样本只证明代码行为；真实源、真实批准、当前 fence、端口纳管和实际观察窗口分别要有证据。以上各层未齐，不能关闭 W17 或把本 Unit 计为已迁移。

## 必须保留的缺源与拒绝语义

当前真实投影仍明确设置 [volume_ratio: None](../../src/market_analyzer/limit_up.rs#L295)，[P-02 selector](../../src/bin/monitor/push_templates.rs#L5964)仍要求有限正量比、有限涨跌幅和有限正价格。保留来源观察不等于获得量比。

- provider 证实空池，只能依照观察对象自身的 `VerifiedEmpty` 事实生成 NoData 证据。
- 非空池却缺量比、字段非法或来源未认证，不能当 VerifiedEmpty；不能把一个 `snapshot: Err(String)` 不加区分地映射为 NoData。
- 全部已通知造成的空选集和缺输入造成的空选集，必须依既有政策/输入合同区分，不能通过错误字符串猜测或把二者都当真实空池。
- 任何新的跨源补量比仍需[量比来源合同](auction-source-evidence-gaps-2026-09-08.md)中列出的产品决定与提供方事实；不得新增隐式 MarketStatistics join，也不得将原 receipt/hash 重新命名为认证。

## 下一任务的可观察验收

验收至少包括：同实例/同次采集；一份横幅；完整提案中消息不变但价格、原始指标、记录顺序、通知集合变化均被拒绝；全量 Ready/NoData/阻断分支的真实 W05 绑定；第二次 provider/账户读取被拒绝；八类实际接入能力的尝试计数；失败不推进通知集合；最终消费与被比较提案一致；受控测试可以 Ready 但真实缺量比不得 Ready。

最终测试命令与文件 ownership 须在具体 adapter/注册 interface 确定后写入正式实施计划，并逐项核对测试是否会调用全局数据库、网络或 dispatcher 日志。当前未运行这些新测试，也未创建这套实现。已有[冻结准备](implementation-auction-frozen-preparation-2026-09-08.md)、[来源保留](implementation-auction-source-observation-2026-09-08.md)、[W17 内核](implementation-w17-results-2026-09-08.md)的完成状态不重开；只补它们之间真实缺失的接线。

其中无需先签发生产权限的输入事实保留已拆出[选集拒绝诊断计划](../superpowers/plans/2026-09-10-auction-selection-diagnostics.md)：将真实选择器/tick的String失败替换为可区分空源、缺字段、有效行已通知的结构化事实，原成功发送结果不变。该计划目前仅准备，须等待唯一实现/Cargo队列交接；它不是完整adapter/注册计划，也不解决真实缺量比或认证。
