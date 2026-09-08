# P-02 来源观察与审计回执保留：实施记录

日期：2026-09-08。状态：**本计划Task1完成；20项lib与51项monitor回归、静态检查及独立评审通过**。源码提交`6b48435`，计划提交`39b2c4f`，原始BASE`250932d`。前批[冻结业务准备](implementation-auction-frozen-preparation-2026-09-08.md)已完成，本批不重复该工作。

## 为什么还需要这一批

前批消息/入池/通知集合已共用准备结果，但基线实际采集路径仍把来源证据投影成普通股票列表。只有文案和股票值，无法让后续影子执行核对它们来自哪次真实请求、哪片名称响应及哪条审计记录。本批已经将同次原始记录和实际审计回执保留到真实竞价消费阶段；这仍是完整来源认证与影子执行的输入，不是它们的替代品。

现有丢弃接点：`src/data_gateway/review.rs:1304`及`:1314`只返回批次，`:678`涨停池调用该路径；`market_capabilities.rs:292`名称也只返回批次。`market_analyzer/limit_up.rs:271`的composition回执仅用于日志，`:303`最终返回普通Vec。`push_templates.rs:6030`及`:6050`只消费Vec，`main.rs:9710`直接拆开tick字段。以上是本批BASE的源码位置，不是生产观测。

## 已实现的数据流与逐点证据

复用真实采集与审计，保留原始涨停池记录、请求hash、批次及回执；名称按实际请求分片保留全部记录、各自证据和回执；保留原composition回执。实际MarketAnalyzer与P02 loader共用它，主入口保留完整tick到P02及持仓消费阶段；旧调用者仍走同一实现，不追加第二次采集或审计。以下位置均对应源码`6b48435`，不是生产运行记录。

| 行为 | 实际代码证据 |
| --- | --- |
| 池采集只写原来的一次审计；旧P01接口投影回原返回值 | `src/data_gateway/review.rs:734,742,784,813`：旧入口委托观察入口，生产与显式DB测试共享请求hash、准入和审计组装 |
| 名称请求保留真实provider、请求hash及receipt | `src/data_gateway/market_capabilities.rs:295,304,324,358`：兼容投影与内部观察共用实际路径，测试仅把审计写往临时库 |
| 原始池、逐片请求/记录/批次、交易日及真实回执只读保留 | `src/market_analyzer/limit_up.rs:23,67`：字段私有，没有公开成功构造器、Default或可变getter；Debug仅类型/状态/数量 |
| 分片仍按原顺序，每片最多50代码；任一片失败立即停止 | `limit_up.rs:307,349,423`：逐片实际代码集/整体覆盖校验、原共享投影与真实分片循环；成功和第二片失败测试调用同一循环 |
| 原有名称语义与空池含义不变 | `limit_up.rs` 的旧 `compose_limit_up_batch` 仍被真实组装调用；逐记录source_at允许与批次不同；空池核对共享投影证据与保留池一致，不请求名称或追加composition |
| 真正进入P02而非仅声明类型 | `limit_up.rs:497` → `src/bin/monitor/push_templates.rs:6059`：真实loader取观察中的stocks准备snapshot，并在同一tick保留Some观察；合成测试入口只保留None |
| main不拆字段后丢弃来源；缺量比时也保留成功取得的观察 | `src/bin/monitor/main.rs:9708,9954`：持有完整tick，借用snapshot/raw列表供P02和持仓检测；其它竞价发送、失败日志和时间窗不改 |

来源观察只代表保存的事实，**不是生产身份认证、发送成功或晋级许可**。既有摘要不改写，新观察不添加量比，不把缺字段补成0。

| 实际来源状态 | 必须保留的业务含义 |
| --- | --- |
| 提供方证实完整空池 | 保留空池采集回执；名称与composition仍0次 |
| 非空池，但量比全部缺失 | 来源仍Available、原股票与证据保留；P02没有有效候选，不能改称VerifiedEmpty |
| 名称或审计失败 | 整批显式失败，不生成观察成功对象或占位回执；不撤销原已写审计 |

## 实际验证与过程修正

最终冻结源码的验证：

- lib：**20 passed / 0 failed / 0 ignored**，3292项过滤；编译1m00s、执行0.55s。六条新审计/观察测试、四条旧组装测试、十条安全Gateway邻域均实际执行。
- monitor：**51 passed / 0 failed / 0 ignored**，680项过滤；编译47.12s、执行0.03s。原消息literal、记录、通知集合、筛选顺序及失败游标断言保留。
- Clippy：exit0、1m42s；159条既有lib警告＋2条既有bin警告，无错误、无改动行诊断；相对前批完整基线无新增，少4条needless-return。不是全仓零警告。
- 五个非main源文件定向格式检查、diff检查和八份冻结RFC输入检查通过。main仅有原始BASE已经存在的8790行格式差异，未重排无关行。
- 独立评审：固定`250932d..6b48435`全任务差异，结论Spec compliant / Approved，无Critical或Important。既有警告和main旧格式差异保留为最终全分支Minor，不以本批通过代替完整项目验收。

交验前修正了测试局部变量遮蔽，并让分片循环和Gateway保留组装真正由生产/测试共用。首次编译发现无PartialEq股票列表断言及显式数据库借用寿命错误，分别改成字段/顺序比较与分支内审计写入；这是编译失败，不是行为RED。后续编译发现空池投影证据未使用，增加其与保留原池的一致性检查，最后重新执行上述20＋51项测试。原始过程输出与最终输出分别保存，没有沿用修正前绿灯。

新用例使用隔离临时SQLite和生产共用组装/审计算法，外部采集仅使用测试替身；没有运行含全局DB初始化、固定路径或网络的整个旧Gateway测试组。bin日志显式隔离到`/private/tmp/p02-source-observation.1wyr8p`。51成员正例断言审计数为池1＋名称2＋composition1，覆盖全部池可选字段及顺序；另覆盖名称错片/重复/漏码、第二片失败、仅composition INSERT失败和默认Debug脱敏。证据位于同计划SDD目录的`task-1-validation.md`、`lib-after-empty-evidence-check.txt`、`bin-after-empty-evidence-check.txt`、`clippy-final.jsonl`及`static-checks.txt`。

## 未解决项与生产边界

[详细计划](../superpowers/plans/2026-09-08-auction-source-observation-retention.md)和[来源合同缺口](auction-source-evidence-gaps-2026-09-08.md)区分可先行工程与量比接源条件。真实竞价量比、生产来源身份、受约束context、完整新旧比较、全局效果纳管、各Unit迁移和发布门禁仍待；不因为本批保留证据而称完整W17完成。

本批不启动、监控、替换monitor，不读取生产数据库、不请求真实provider或发送消息，不修改冻结蓝图及其他输入。
