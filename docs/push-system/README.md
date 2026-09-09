# 推送系统文档入口

状态：`PROVISIONAL`。本批输入冻结日期为 `2026-09-06`。源码接线、历史统计、原工作区文档和拟议设计必须分开阅读；它们不等于部署证明、远端 `TransportAccepted` 或用户已读。

## 当前开发入口（更新至2026-09-10）

最新进度：后续蓝图发现的MU-auction-volume旧摘要已由当前审计Task2在047b4ab修正；真实draft0、strict仅七项发布条件、610项只读与独立Spec/Quality均通过，新增问题已关闭。新版蓝图将使用最终SHA继续，不混用下方Task1的ff94eca身份。另撤回“v18/v19实际仅九份文件”的结论：Git跟踪16份，其中九份属于固定source catalog，另七份已完整读取并单列原文证据，不扩张冻结来源权威。

2026-09-10：完整[当前架构蓝图](../architecture/current/Project_Architecture_Blueprint.md)已在c1e24d0交付Markdown，覆盖18节、65/102/52精确身份及16份设计来源，独立Spec/Quality Approved；[实施记录](implementation-current-blueprint-2026-09-10.md)区分已验证内容与两处后续小修。现在转入双HTML开发，尚无第二HTML或生产迁移完成证明。

- [统一文档门禁实施记录](implementation-unified-document-checker-2026-09-09.md)：最终源码7a150b2，限定复审Approved；enum失败漏strict/无ID重复错误、root参数及额外别名全部关闭。修复后定向2/65、1/17、3/28通过；实际draft107/strict112和598项只读证明重新取得。首次TDD过程例外及初审描述更正均保留，不冒称当前draft或远端CI通过。合同见[计划](../superpowers/plans/2026-09-09-unified-document-checker.md)。
- [强制当前源码审计实施记录](implementation-current-source-audit-2026-09-09.md)：初版c2e33a2、最终修复ff94eca；历史/current两层同批接通，548文件/250声明、65 kinds/102 producers/52 Units身份保持。Catalog整套48/620及最终定向通过；checker原20例有1个测试预期错误，修正后单例10断言通过。独立初审发现的集合竞价摘要遗漏已修复，限定复审Approved、无开放问题。修复后真实draft0、strict仍为6项provisional+提交前dirty，610项只读证明通过；仅此审计Task完成，不代表发布或迁移完成。[计划](../superpowers/plans/2026-09-09-current-source-audit.md)保留全部合同。
- [当前Unit摘要后续修复](implementation-current-source-audit-2026-09-09.md#task2-unit层残余说明纠正)：047b4ab仅改三current制品4+/4-，修复主循环/dispatcher内部推进与双次采集旧文案；独立Spec/Quality通过，原字节绑定/正式派生和实际树验收齐全。此条为最终current制品入口，历史规范不变。
- [当前蓝图与双目标HTML计划](../superpowers/plans/2026-09-09-current-blueprint-offline-html.md)：Task1当前Markdown已交付并审查；Task2继续第二target/wrapper和实际离线浏览器验收，新路径不覆盖冻结蓝图。
- [当前代码审计增量](current-code-audit-delta-2026-09-09.md)：固定aef7972；107条原始漂移已归类，新增79文件含46生产/33测试，9+29候选已作为后续正式current审计输入。最终current材料见上方ff94eca实施记录，原增量文档保留其调查时点和范围，不改旧RFC/WBS/runtime目录。
- [当前蓝图规模、调用链与运行边界核对](current-blueprint-inventory-2026-09-09.md)：全仓594个Rust文件/445884行、62个公开顶层模块，与push审计548项口径分开；补核认证/DB/配置/CI、CLI与数据平面、两套Foundation存储、依赖与测试证据边界。四时段主归属按规范10/6/21/28计数，不沿用旧蓝图小标题。targets仅静态候选，无metadata或生产验证，不代表新版蓝图/第二HTML已完成。
- [RFC 离线HTML构建](implementation-offline-rfc-html-2026-09-09.md)：最终源码ca1b581；23项测试/676条断言通过，独立限定复审Approved，原3项及段落边界回归均关闭。实际浏览器4图成功/1图回退、点击与注入阻断、全屏和页面0外部请求通过；[离线页面](push-system-implementation-rfc.html)已纳管。保留官方发行JS原字节及30条空白告警，不覆盖八份冻结输入，不代表双份HTML/统一checker/CI或当前证据目录已完成。
- [旧库升级与恢复审计接续](implementation-durable-upgrade-2026-09-08.md)：源码77cc3bc；正式升级/提交、原rowid与数据保持、正确接续/终态、重复恢复/实例重开及坏FK完整回滚，最终67项合批、静态检查及独立规格/质量审查通过。已被旧迁移重排的v5–v9库保留独立兼容任务，不代表全部审计兼容或全项目完成。
- [恢复分类不确定投递读取修复](implementation-recovered-uncertain-read-2026-09-08.md)：源码a2429b3、修复fb55d32/61e9d16；真实过期恢复贯通Generic/P01及SLA/指标。自环与跳链均实际RED后修复，最终55项/静态检查/限定复审通过；无盲重发或状态提升，不等于完整Q39/W19完成。
- [CI 映射式触发识别修复](implementation-ci-mapping-trigger-2026-09-08.md)：源码80d0fb2；95项定向测试、614条断言通过，独立复审已关闭排除项取消全部正向匹配漏洞；识别当前真实CI触发格式，不改CI配置，也不代表HTML/统一checker/实际CI已交付。
- [52个迁移单元的当前完成证据](remaining-migration-evidence-2026-09-08.md)：区分目录覆盖、6个已核对旧业务入口、46个本次未逐链核对单元与完整迁移认证；不以注册表或测试数量计算迁移完成率。
- [P-02 来源观察保留实施记录](implementation-auction-source-observation-2026-09-08.md)：本计划完成，20项lib与51项monitor回归、Clippy及独立评审通过。实际采集/名称分片/审计回执保留到竞价tick消费，旧投影兼容；非空池缺量比不称VerifiedEmpty，不增加量比来源或生产权限，不等于完整W17完成。
- [P-02 冻结业务准备实施记录](implementation-auction-frozen-preparation-2026-09-08.md)：源码`eeb2ddc`、测试修复`74954fe`；一次横幅捕获、完整消息/逐票记录/通知集合提案已被实际dispatcher消费。修复后51项测试、静态检查及限定复审通过；没有补造量比来源，不代表完整W17迁移或上线完成。
- [P-02 量比与来源证据核对](auction-source-evidence-gaps-2026-09-08.md)：现行规则禁止跨批补量比，MarketStatistics同名字段尚无完整竞价合同；区分可先行的真实证据保留工程与需要产品/提供方确认的接源条件，不改变生产来源。
- [W19 全量库存指标实施记录](implementation-w19-inventory-results-2026-09-08.md)：源码8094ffc、修复c10ec78；覆盖指定namespace全部业务日/状态、真实来源SLA、错误分母与扫描预算。N02缺lock冷读与Completed+Conflict漏等待年龄两个反例先RED再修复，最终107项测试/静态检查/限定复审通过。readiness/晋级消费者、完整留存与安全审计仍待，不将库存指标当生产健康许可。
- [W19 持久化最终化延迟检查结果](implementation-w19-results-2026-09-08.md)：初版27967be、修复6f8713c；通过Generic/P01/N02实际持久化reader与同事务业务全链读取，使用原始接受时间计算两周期目标/五分钟硬上限，保留人工、未决和冲突状态。审查发现的历史终态/资格遗漏先实际复现13种矛盾再修复，最终78项测试通过、本批静态零诊断，限定复审全部关闭；仅N02明确支持的局部occurrence约定，不代表生产注册。完整W19的指标汇总、留存与安全审计及生产消费仍待。
- [W17 影子执行内核结果](implementation-w17-results-2026-09-08.md)：源码`89128f3`，一次采集共用context/facts，精确比较真实决策/语义/渲染字节/完成提案；八类拒绝端口先计数再拒绝。17项新测试和52项相邻测试全部通过，最终静态检查本批零诊断，独立Spec/Quality通过。生产业务适配、全局端口纳管及W16激活证据仍待，不代表完整W17或迁移完成。
- [W15 实施结果](implementation-w15-results-2026-09-07.md) §10–12：依赖候选合同、实际 occurrence 读取和采集事务适配器已通过限定验证/审查，完整来源认证、运行查询及调度联结尚未完成。
- [W16 实施结果](implementation-w16-results-2026-09-08.md)：已有跨进程broker接入真实initial写入、Generic发送与只恢复；最新T4D将精确业务恢复/finalizer写入接入同一worker许可（源码`4c07aaa`、测试修正`667ee4a`）。修正后受影响合批104项通过，含4个真实进程父测试，目标Clippy零诊断，限定复核全部关闭；未变邻域保留此前498项通过证据，不冒称修正后全量重跑。生产身份/受保护根、完整四actor共同fence、实际切换、具体Unit cursor及全Unit消费仍按[实施计划](../superpowers/plans/2026-09-08-push-foundation-w16-activation.md)交付，不能按基础切片标W16完成。
- [W16 合同裁决](activation-contract-decisions-2026-09-08.md)：澄清 shadow actor 与旧推送关系，固定外部批准包、同事务写入和paused确认顺序；真实监督器及恢复接线仍需实现证明，不是生产批准。
- [全 Unit 部署集合合同](activation-deployment-set-contract-2026-09-08.md)：全52Unit候选集合构造、真实读取、重读漂移和范围投影已实现至`e0cdd0d`，11项专项及最终golden1项通过，限定复核关闭排序缺陷。集合版持久消费后续进展见下方计划入口及[实施结果](implementation-w16-results-2026-09-08.md)，实际来源认证仍未交付。
- [P-02 同批数据修复结果](implementation-auction-volume-results-2026-09-08.md)：源码`0dda4cc`将选票、消息、入池和通知set绑定到一次采集的选中快照，同时保留持仓检测使用的完整原始列表。13项相关测试通过，最终静态检查在本次两份改动文件零诊断，独立规格/质量审查通过，无待修问题；[实施计划](../superpowers/plans/2026-09-08-auction-volume-batch-binding.md)的完整Unit迁移边界保持不变，生产monitor不替换。
- [集合快照与恢复存储接线计划](../superpowers/plans/2026-09-08-readiness-deployment-set-v3.md)：源码`4a3ef37`已将全Unit集合接入snapshot/material v3、stream v2及既有恢复/store链，补齐Core恢复责任，保留旧v2及冻结event/schema。首轮60通过/2失败，修正临时路径后新模块5/5通过，旧57项证据保留，最终Clippy目标零诊断；独立规格/质量审查通过，保留版本冲突直接测试及既有warning两项Minor，不代表实际source认证、query/probe或完整T6完成。

以下第三批状态是 **2026-09-06 的历史交接快照**，其中“W01--W21 运行时未实现”等描述不代表上述后续实现进度；原始输入/权限边界仍有效。

## 第三批历史交接

第三批规格完成：RFC/SQL/WBS 规格与机器合同已通过最终独立审查，原审查 findings 全部关闭；这不是推送系统改造完成。[第三批交接结果](implementation-batch-3-results-2026-09-06.md) 记录 FINAL SPEC PASS（0/0/0）、wave3 FINAL SCOPED QUALITY PASS（0/0/0）的精确范围，以及 clean `e35800a` 的 418 runs / 5023 assertions 和临时 SQLite 证据。最终状态提交未重跑五套测试，规格完成不提升 `PROVISIONAL` 发布状态。

## 事实边界与阅读顺序

1. [已批准决策](grill-decisions-2026-09-02.md) 与 [设计来源目录](../../design-source-catalog.v1.json) 记录设计约束及来源裁决。
2. [65-kind 能力目录](push-capability-catalog.md)、[机器目录](push-capability-catalog.v1.json) 和 [源码证据 manifest](push-evidence-manifest.v1.json) 记录隔离分支在 Rust 基线 `07781bf386aafdf202851ae928efee8920387058` 的源码接线。
3. [RFC 输入 manifest](rfc-input-manifest.v1.json) 冻结下列八份原工作区输入的原始字节、尺寸、SHA-256、角色与冲突。冻结不提升其事实权限。
4. [实施 RFC](push-system-implementation-rfc.md) 定义身份、应用结果、完成权威、跨库恢复、调度/readiness、shadow/activation/operator、保留期与验收合同；[独立 SQLite CLI 规格](push-system-foundation.v1.sql) 与 RFC 嵌入逐字节一致，仅在临时数据库验证。
5. [WBS 机器事实源](push-system-wbs.v1.json) 定义重建的 W01--W21、52 Unit 估算/依赖/门禁，RFC 内只保留生成摘要。828.99h 基线、994.79h 缓冲后工时及条件日历场景是可复算估算，不是承诺日期。[第三批实施计划](implementation-batch-3-rfc-wbs-2026-09-06.md) 和交接结果记录最终独立审查结论与本批明确不交付的边界。

严格 RFC 门禁仍返回 `rfc_status_provisional`、`wbs_status_provisional`、`rfc_html_missing`、`ci_rfc_gate_missing`；显式 `check-catalog.rb --root . --check` 仍返回 catalog/manifest 两项 provisional。W01--W21 运行时未实现，52 Unit 未迁移、未 shadow/live promote；Rust、生产 DB schema、配置、模板及业务行为未改。蓝图 §24/§25 去拟议化、通用离线 HTML builder、RFC HTML 与统一 checker/CI 接线未交付。未部署，无真实接收、用户已读或交易收益证明；PaperBuy/Watchdog 仍为原混乱工作树排除项，160 个冲突未解决。

## 八份不可变输入

| 输入 | 角色与限制 |
| --- | --- |
| [架构蓝图 Markdown](../Project_Architecture_Blueprint.md) | 当前架构与拟议设计的语义输入快照，原文的 CURRENT 不直接证明本隔离分支或部署状态。 |
| [架构蓝图 HTML](../Project_Architecture_Blueprint.html) | `derived visual snapshot`，不是独立语义真相，也不是本批已重建的 HTML。 |
| [历史再分析](comprehensive-reanalysis-2026-09-05.md) | 2026-08-31 至 09-04 生产证据分析、风险与验收样本。 |
| [聚合证据 JSON](recent-push-evidence-2026-09-05.json) | 脱敏历史快照，不等于实时生产状态或跨库原子快照。 |
| [原 67-kind 视图](all-push-kinds-2026-09-05.md) | `non-baseline evidence`，不覆盖本分支 65-kind 能力目录。 |
| [原文档硬化计划](push-documentation-hardening-plan.md) | 验收要求及后续 HTML/CI 边界输入，不是当前执行完成记录。 |
| [再分析程序快照](reanalyse-recent-pushes.rb) | 派生程序快照，本任务不执行生产读取。 |
| [内部一致性程序快照](verify-reanalysis.rb) | 证据内部一致性校验快照，本任务不执行。 |

67 与 65 的差异不通过补造能力消除：`PaperBuy` / `Watchdog` 在原混合工作树存在，本隔离分支未移入，仅保留为 `excluded_worktree_additions`；不得据此创建本分支 MigrationUnit 或激活证明。原快照中的状态、规模和历史行号不是当前源码证据身份。

## 离线输入门禁

在仓库根执行：

```bash
ruby scripts/architecture-docs/check-rfc-inputs.rb --root .
ruby scripts/architecture-docs/test/rfc_inputs_test.rb
```

门禁仅校验 manifest 的契约、相对路径、普通文件身份、原始字节尺寸与 SHA-256，不写入或修复输入；读取前检查真实路径包含关系，拒绝目录及任何输入路径链接。成功返回 `0`，内容失败返回 `1`，参数失败返回 `2`。`PROVISIONAL` 在此仅是输入 manifest 的固定状态，校验成功不是发布门禁通过。

不得对八份输入做换行规范化、格式化或重生成。两份导入 Ruby 只执行 `ruby -c` 语法检查；不要把本入口的导入/校验步骤变成生产查询或脚本重跑。
