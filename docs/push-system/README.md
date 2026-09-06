# 推送系统文档入口

状态：`PROVISIONAL`。本批输入冻结日期为 `2026-09-06`。源码接线、历史统计、原工作区文档和拟议设计必须分开阅读；它们不等于部署证明、远端 `TransportAccepted` 或用户已读。

## 事实边界与阅读顺序

1. [已批准决策](grill-decisions-2026-09-02.md) 与 [设计来源目录](design-source-catalog.v1.json) 记录设计约束及来源裁决。
2. [65-kind 能力目录](push-capability-catalog.md)、[机器目录](push-capability-catalog.v1.json) 和 [源码证据 manifest](push-evidence-manifest.v1.json) 记录隔离分支在 Rust 基线 `07781bf386aafdf202851ae928efee8920387058` 的源码接线。
3. [RFC 输入 manifest](rfc-input-manifest.v1.json) 冻结下列八份原工作区输入的原始字节、尺寸、SHA-256、角色与冲突。冻结不提升其事实权限。
4. [第三批实施计划](implementation-batch-3-rfc-wbs-2026-09-06.md) 定义后续 RFC/WBS 工作；本次导入不代表完整 RFC、SQL、WBS、HTML、CI 或运行时 Foundation 已实现、已验收，也不构成工期验证。

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
