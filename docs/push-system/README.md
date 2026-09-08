# 推送系统文档入口

状态：`PROVISIONAL`。本批输入冻结日期为 `2026-09-06`。源码接线、历史统计、原工作区文档和拟议设计必须分开阅读；它们不等于部署证明、远端 `TransportAccepted` 或用户已读。

## 当前开发入口（2026-09-08）

- [W15 实施结果](implementation-w15-results-2026-09-07.md) §10–12：依赖候选合同、实际 occurrence 读取和采集事务适配器已通过限定验证/审查，完整来源认证、运行查询及调度联结尚未完成。
- [W16 实施结果](implementation-w16-results-2026-09-08.md)：已有跨进程broker接入真实initial写入、Generic发送与只恢复；最新T4D将精确业务恢复/finalizer写入接入同一worker许可（源码`4c07aaa`、测试修正`667ee4a`）。修正后受影响合批104项通过，含4个真实进程父测试，目标Clippy零诊断，限定复核全部关闭；未变邻域保留此前498项通过证据，不冒称修正后全量重跑。生产身份/受保护根、完整四actor共同fence、实际切换、具体Unit cursor及全Unit消费仍按[实施计划](../superpowers/plans/2026-09-08-push-foundation-w16-activation.md)交付，不能按基础切片标W16完成。
- [W16 合同裁决](activation-contract-decisions-2026-09-08.md)：澄清 shadow actor 与旧推送关系，固定外部批准包、同事务写入和paused确认顺序；真实监督器及恢复接线仍需实现证明，不是生产批准。
- [全 Unit 部署集合合同](activation-deployment-set-contract-2026-09-08.md)：全52Unit候选集合构造、真实读取、重读漂移和范围投影已实现至`e0cdd0d`，11项专项及最终golden1项通过，限定复核关闭排序缺陷。后续snapshot/material v3、stream v2消费与实际认证仍未交付，详见[实施结果](implementation-w16-results-2026-09-08.md)。
- [P-02 同批数据修复结果](implementation-auction-volume-results-2026-09-08.md)：源码`0dda4cc`将选票、消息、入池和通知set绑定到一次采集的选中快照，同时保留持仓检测使用的完整原始列表。13项相关测试通过，最终静态检查在本次两份改动文件零诊断，独立规格/质量审查通过，无待修问题；[实施计划](../superpowers/plans/2026-09-08-auction-volume-batch-binding.md)的完整Unit迁移边界保持不变，生产monitor不替换。
- [集合快照与恢复存储接线计划](../superpowers/plans/2026-09-08-readiness-deployment-set-v3.md)：正在把全Unit集合接入snapshot/material v3、stream v2及既有恢复/store调用链，并补齐Core恢复责任范围。保留旧v2字节与冻结event/schema；此片尚未验证，不代表实际source认证、query/probe或完整T6完成。

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
