# Durable 运行期数据库版本防护

日期：2026-09-10。状态：本 Task 完成；23 项定向测试及静态检查通过，独立 Spec/Quality Approved。唯一报告摘要 Minor 已更正并复核，既有告警保留。固定 BASE：`8daa8bf2d8490807256a5bb6ed92b0a4ea8a5734`；SOURCE：`8ee1e13fa77895fcb2633a2da2d0c6824aba2042`。验证与完整目标边界见[实施记录](../../push-system/implementation-durable-runtime-schema-guard-2026-09-10.md)。

## 目标和依据

正式 open 会拒绝高于支持版本的数据库，但已打开 coordinator 的通用操作没有同等检查。`validate_persisted_immutable_references` 在版本不等于当前版本时返回 Ok，原本用于尚未完成迁移的 bootstrap；运行期也会沿用这个跳过规则。本批将初始化与运行期区分，阻止可观察的版本漂移绕过操作校验，不改变审计排序或历史恢复合同。

## Global Constraints

- 唯一项目工作目录为 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`。沿用现有隔离分支；不读取根 checkout 项目内容，不操作生产 monitor、实际配置/.env、真实数据库/业务数据、provider/LLM/sink/订单/PAM、凭据、网络、部署或远端 Git/CI。
- 唯一实施 agent 修改授权 Rust 文件和本计划专属报告；父线独占 Cargo/Git/中文 docs/SDD 记录。代理不启动 Cargo、不提交、不派子代理。所有源文件编辑使用 apply_patch，定向格式化除外。
- 测试仅用自有 `data/test/TEST_CODE_*` 或内存 SQLite 及受限 MemoryAppendPort。既有 Fixture 只 stat 隔离树下生产命名对象以检查不变性，不读取这些对象中的数据库内容；清理保留精确 inode 归属边界。
- 不改 schema DDL、SCHEMA_VERSION=9、历史迁移算法、canonical/hash、外部公开 API、冻结 Foundation SQL、八份 RFC 输入、catalog 或业务审批规则。不把本修复表述为旧版本二进制撤销、完整审计兼容或生产上线。

## Task 1: 区分 bootstrap 与 runtime，并在实际操作边界拒绝版本漂移

### 文件与所有权

- 唯一 Rust writer 可修改 `src/durable_delivery/coordinator.rs` 和 `src/durable_delivery/tests.rs`，限本任务运行期版本校验及定向回归。`schema.rs`/`model.rs` 只读。
- 父线维护本计划、专属 `.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/`、中文实施记录及索引。

### 明确合同

1. 成功打开后，数据库 `user_version` 必须等于 `SCHEMA_VERSION`；较低版本（含 0）和较高版本均不允许通用运行期 API 继续 callback。不在 runtime 自动迁移或修补版本。
2. bootstrap 保留真实空库初始化与已支持旧版迁移，采用私有且只能由 open 使用的明确入口/策略，复用现有连接/事务/回滚引擎。不要以可公开设置的 allow-legacy 参数、全局可变开关或长期 permissive coordinator 模式绕过运行期规则。
3. runtime 连接 callback 前检查当前版本；写事务在 BEGIN IMMEDIATE 后、callback 前再次检查，防止外层检查与获取写锁之间的版本变化。正常完成后也检查版本，读操作不得在已观察漂移后返回成功。
4. 所有写事务（包括 schema bootstrap）提交前必须处于当前版本；callback 内产生版本漂移时，拒绝并显式回滚原业务数据、审计和版本变更。保持现有 primary/rollback/post-isolation 组合错误证据，不吞掉回滚失败或把 COMMIT 后错误说成已经回滚。
5. 正式 open 返回 coordinator 前，在既有最终 attested lease 内验证当前版本。较新版本 open 仍明确拒绝；不以仅内存 initialize_test_schema 测试替代真实 open 的关键合同。
6. 已存在的可见漂移导致 prepare/reconcile 拒绝时，不产生该调用的数据库业务变化或新外部 append。此项不保证任意外部 SQL 管理员短暂改版又恢复均可观测，也不保证整个多事务 reconcile 对并发漂移具有外部效果原子性；已经成功的外部 append 无法倒退。
7. 不更改 Pending/Appended、严格 uncertainty inspector、审计逻辑尾选择或合法跨 decision 恢复语义。未补齐的 v5–v9 历史顺序来源、旧 writer 正式排空仍另列未完成。

### TDD 与验收

- 第一条真实反例：通过 Fixture 的正式 open 建立当前版本数据库及实际 decision，用另一条仅指向该 TEST_CODE 数据库的 SQLite connection 改 `user_version`；真实 `decision_state` / `prepare` 在漂移后必须拒绝，原实现实际会继续。测试-only 冻结，通知父线运行并取得预期行为 RED 后，才修改生产代码；编译失败不计 RED。
- 覆盖 0、较低受支持版本及高于当前版本；使用新测试前缀 `runtime_schema_guard_`，不要修改旧例预期来配合新实现。
- 覆盖 runtime callback 前可见漂移、外层校验后/事务获取后重验（需要时增加最小 cfg(test) 注入点）、同一写事务内漂移导致完整回滚、读操作后验漂移拒绝，以及最终 open 成功边界版本漂移拒绝。钩子仅注入故障，不提供生产权限或新通用执行入口。
- 对初始已漂移的实际 `reconcile_all_pending` 检查 MemoryAppendPort 记录不变、业务与 audit 快照不变；通过恢复版本后真实读取/重试验证正常功能，必要原始 SQL 快照用于公开 API 故意拒绝时检查回滚，不只测新私有 helper。
- 正向回归保留正式新建、同版本重开、第二 coordinator 使用，以及既有真实 v4 升级、失败 bootstrap 回滚、旧 schema 迁移矩阵、immutable-reference 校验失败回滚、复合 commit/rollback 错误和 W16 跨 decision Pending 全局恢复。父线先核对每条测试副作用，仅运行选定离线测试，不运行未经审计的全库 suite。
- Cargo 由父线单队列运行，固定 cwd、`--offline --lib`、精确测试名/前缀；保存每条命令完整 stdout/stderr、真实 exit/session。基线、RED、GREEN 分文件记录，禁止覆盖或猜测 session。
- 最终源文件格式化完成后冻结；父线在验证前后记录 SHA-256，无改动期间只运行一次有效最终定向集合。最后运行离线 lib Clippy、定向 rustfmt 检查与 git diff --check，旧 warning 与新诊断分开，不加 allow 隐藏。
- 父线提交本 Task 源码，固定 BASE..SOURCE 独立做一次规格与质量审查；发现问题由原 writer 修复，并只对修复范围复审。有效测试证据不因写文档或审查重复跑。

## 完成边界

本批只是未来 schema 扩展前所需的已打开新版 coordinator 防护。它无法改写已经部署的旧二进制，也不建立 v10 历史来源、恢复旧审计缺口或提供生产切换授权。P02 真实绑定、W16 生产认证/栅栏、52 Unit 接入、剩余运维验收和完整目标继续保留。
