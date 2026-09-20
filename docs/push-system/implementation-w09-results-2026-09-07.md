# 推送 Foundation W09 实现与验证结果

**结论：** W09 已实现 `VerifiedTerminalRef` 的受控构造和 finalizer 前二次重验证。强终态引用只能来自按精确 `decision_id` 查询的私有 authority 端口；验证器会重新校验 W08 intent、完成策略、authority descriptor、全部 `TerminalBinding/v1` 字段、原始 evidence bytes 及两层 SHA-256。finalizer 只能取得二次查询后生成、不可 Clone 的 fresh capability。兼容层弱 audit 没有进入该构造链的类型或转换入口。

**生产边界：** 本切片没有实现生产 durable adapter、业务 finalizer、状态 CAS 或 Unit 接线，没有修改 monitor、notification、durable delivery、配置、Cargo 清单和冻结 DDL。W10--W21 与 52 个 Migration Unit 仍未完成；线上推送继续由原 release monitor 执行。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 验收目标与代码证据

WBS W09 的验收句是：“构造和 finalize 时重验 authority/decision/bytes/subject 全部绑定，弱 audit 永不冒充 receipt。”实现逐点对应如下：

| 验收点 | 实现结果 | 代码证据 |
| --- | --- | --- |
| 构造时重新查询 | `verify_terminal` 从再次验真的 Ready intent 派生 decision，只按该 key 调用一次 authority | `terminal_authority.rs:176-209`、`intent_store.rs:591-635` |
| finalize 时重新查询 | `reverify_for_finalization` 不复用旧查询结果，再次执行完整 `verify_terminal` | `terminal_authority.rs:292-305` |
| authority 绑定 | 完成策略必须允许 descriptor authority；record 的 authority/schema 还必须与 descriptor 一致 | `terminal_authority.rs:189-220`、`policy.rs:532-542` |
| decision 绑定 | 查询 key 由 attested intent 的稳定 decision 派生；record decision 再与预期逐项比较 | `intent_store.rs:617-633`、`terminal_authority.rs:202-224` |
| bytes 绑定 | rendered SHA 与 W08 保存值比较；evidence 原始字节不能为空并现场重算 SHA | `terminal_authority.rs:240-251` |
| subject 绑定 | record subject 与重建后的 intent subject 使用 typed value 精确比较 | `terminal_authority.rs:222-244` |
| 全字段绑定 | `TerminalBinding/v1` 固定 17 项材料，重算后与 authority 声明摘要比较 | `terminal_authority.rs:262-265,318-401` |
| 旧引用漂移阻断 | prior/fresh 比较覆盖全部稳定字段，只排除重新查询时间 `verified_at` | `delivery.rs:222-241`、`terminal_authority.rs:300-304` |
| 弱 audit 隔离 | authority query 只有 Missing、PendingSeal、Terminal；无 compatibility variant，强引用 parts 构造仍为 crate-private | `terminal_authority.rs:113-142,267-289`、既有 rustdoc `compile_fail` |

## 2. 权限与类型边界

`TerminalAuthorityPort`、`AuthorityDescriptor`、`AuthorityTerminalRecord`、`AuthorityQuery`、验证函数和 finalization capability 都是 `pub(crate)`。外部 producer、兼容通知路径或 CLI 不能自己拼 authority record，也不能直接调用 parts constructor 伪造 `VerifiedTerminalRef`。

公开新增的 `TerminalTemplateBinding` 只接受 typed `TemplateId` 与 `TemplateVersion`，并按 `TemplateBinding/v1` 计算 SHA。W08 intent 保存的 immutable `template_sha256` 必须与它一致；不一致时在查询 authority 之前即失败。这样 template ID/version 不能只凭相同标签或 authority 自报值放行。

`AuthorityQuery::Terminal` 对 record 使用 `Box`，避免整个查询枚举被大记录拖成大栈对象。此项来自 strict Clippy 的目标内反馈，不改变查询或验证语义。

## 3. 精确 TerminalBinding 合同

canonical domain 固定为 `TerminalBinding/v1`，材料恰为：

```text
ref_id, authority_class, namespace, decision_id, attempt_id,
intent_id, unit_id, occurrence, business_date, subject, audience,
template_id, template_version, rendered_sha256, terminal_disposition,
evidence_sha256, durable_schema_version
```

`verified_at` 和 `binding_sha256` 自身明确排除。namespace、subject 复用 W01 的 canonical 表示，attempt 缺省值编码为 JSON `null`，所有字段通过 `BTreeMap` 进入现有 canonical-v1 编码。golden test 独立拼出完整 preimage，再用原始 SHA-256 计算期望值，避免生产 helper 自证。

验证顺序是 fail closed 的：

1. 重新运行 W08 `verify_snapshot`，只接受完整 Ready intent；
2. 核对 template binding 与完成策略 Unit/owner；
3. 检查 authority 是否被策略允许；
4. 按 intent 派生的 exact decision 查询；
5. 核对 descriptor 与 record authority/schema；
6. 逐项比较全部业务预期字段；
7. 拒绝空 evidence，并从 exact bytes 重算 evidence SHA；
8. 验证 disposition/attempt 的受控组合；
9. 重算 `TerminalBinding/v1` SHA 并比较声明值；
10. 最后才消费 record 构造不透明强引用。

任一环节失败都只返回稳定错误分类或字段名，不回显 subject、渲染正文、receipt bytes、数据库路径或 SQL。

## 4. disposition 与 attempt 语义

RFC 允许 `attempt_id` 仅在“经校验的尝试前拒绝或人工处置”时为空。实现没有用裸 `Option<AttemptId>` 接收这个权限，而是使用受控枚举：

- `Attempt(AttemptId)`：真实 attempt，可用于五种处置；
- `ValidatedPreAttemptRejection`：只允许 `Rejected`；
- `ValidatedManualWithoutAttempt`：只允许 `ManualConfirmedAccepted` 或 `ManualConfirmedNotDelivered`。

因此 Accepted/Uncertain 不能无 attempt，普通 `None` 也不能冒充受校验例外。完成投影保持 W02 合同：Accepted -> `TransportAccepted`，Rejected -> `TransportRejected`，Uncertain -> `TransportUncertain`，两种人工处置 -> `AlreadyTerminal`；人工接受不会伪装为传输接受。

## 5. finalize 二次重验证

首次 `verify_terminal` 返回可供应用观察/投影的 `VerifiedTerminalRef`，但 W10 不能直接把它当最终化权限。`reverify_for_finalization` 会：

1. 对同一 attested snapshot、template、policy 和 authority 再执行一次完整查询与校验；
2. 比较 prior 与 fresh 的 ref、authority、decision、attempt、intent、Unit、occurrence、日期、subject、audience、template、rendered/evidence SHA、disposition、schema 和 binding SHA；
3. 只允许 `verified_at` 因二查时间变化；
4. 返回不可 Clone、按值消费的 `FinalizationTerminalRef`。

两次查询之间 authority 变成 missing/pending/unavailable、换 ref、换处置或任一稳定字段漂移时，都不会产生 finalization capability。W10 仍需实现消费 capability 的同库业务 CAS，这不属于 W09。

## 6. TDD 与评审提交

| 提交 | 内容 | 证据 |
| --- | --- | --- |
| `6d3a876` | W09 设计与逐测试计划 | 冻结完整 binding、二查和零接线边界 |
| `8cc8d76` | 第一组 RED | authority/binding/verifier 尚不存在，按预期编译失败 |
| `69b7a78` | 首次验证 GREEN | exact query、全字段/hash 校验、强引用构造通过 |
| `7aeccbc` | 第二组 RED | `reverify_for_finalization` 尚不存在，按预期编译失败 |
| `1883ba8` | finalize 二查 GREEN | fresh capability 与 prior drift 阻断通过 |
| `15e88ca` | 双轴 review 修复 | 收紧 attempt 例外、空 evidence、全业务字段变异与 finalize 全量重验 |
| `8602b16` | target lint 修复 | boxed terminal query record，目标 strict lint 清零 |

评审后没有遗留 W09 critical/high/medium finding。具体生产 durable adapter 被明确留给 W12，业务 CAS/finalizer 被明确留给 W10；本切片没有用 fake adapter 或 public parts constructor 越过依赖顺序。

## 7. 行为测试覆盖

W09 共 12 条目标测试：

1. `TemplateBinding/v1` 与 `TerminalBinding/v1` exact golden preimage/hash，且 `verified_at` 不入摘要；
2. 首次构造只查询 exact decision，并映射强结果；
3. authority、decision、rendered bytes SHA、subject 分别漂移都阻断；
4. evidence bytes/hash 与声明 binding hash 均现场重算；
5. namespace、intent、Unit、occurrence、日期、audience、template ID/version、schema 逐项变异；
6. template mismatch 与空 evidence 是两个独立门禁；
7. attempt/尝试前拒绝/人工处置的类型组合及五种结果映射；
8. missing、pending、unavailable、disallowed authority 全部失败关闭；
9. completion policy Unit/owner 在 authority 查询前绑定；
10. finalize 成功路径查询总次数精确为两次，只返回 fresh 时间；
11. authority 替换稳定 ref 时返回 `PriorReferenceChanged`；
12. finalize 二查重复全部绑定校验，subject 漂移直接失败。

既有 `CompatibilityEvidenceRef -> VerifiedTerminalRef` rustdoc `compile_fail` 同时保留，证明弱 evidence 在编译期没有强引用转换入口。

## 8. Fresh 验证

| 门禁 | 结果 |
| --- | --- |
| `cargo test --lib w09_ -- --test-threads=1` | PASS：12 passed / 0 failed |
| `cargo test --lib push_foundation -- --test-threads=1` | PASS：35 passed / 0 failed（W07 9 + W08 14 + W09 12） |
| `cargo test --lib monitor::push_job::tests -- --test-threads=1` | PASS：52 passed / 0 failed |
| `cargo test --doc` | PASS：16 passed / 0 failed / 4 ignored，含弱 evidence 编译失败门禁 |
| `cargo check --lib --message-format=short` | PASS；84 个目标外既有 warning；无 W09 warning |
| strict Clippy | 基线阻断：163 个目标外既有 lint；首个在 `src/data_gateway/futures_delivery.rs:15`；W09/push_foundation/push_job 零错误 |
| nonfatal Clippy | PASS，exit 0；163 warnings，W09 目标零 warning |
| 定向 rustfmt | PASS：W09 文件及模块根无格式差异 |
| architecture docs 五组验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| `git diff --check 1f222ae..HEAD` | PASS |
| production-wiring relative diff | PASS：monitor、notification、durable delivery、config、Cargo manifests、冻结 SQL 相对 W08 均无变化 |

strict Clippy 的 163 项是本次 fresh 工具链报告的目标外既有基线；本切片修复了唯一 W09 大枚举错误，没有修改不相关旧代码，也没有把 strict 全仓描述成通过。

## 9. 生产 monitor 只读观察

2026-09-07 07:12 CST 只读观察：既有 `./target/release/monitor` 仍为 PID 20162；进程继续持有根仓库 release 二进制、业务库、durable 库和 production delivery lock。`10.211.55.2:60076 -> 10.211.55.3:50051` 与 `127.0.0.1:54144 -> 127.0.0.1:18082` 都为 ESTABLISHED。

`data/push_log/2026-09-07` 仍不存在。此时尚在盘前，日志缺失本身不是故障；TCP 连接和进程存活也不是业务消息已 accepted 的 authority 证据。本次 W09 开发没有重启、重建、热替换或修改该生产进程。

## 10. 尚未完成

- W10 通用 finalizer：消费 fresh capability，在业务库执行完成/不投递 CAS，冲突进入 `ResolutionRequired`；
- W11--W21：恢复器、调度、readiness、activation、shadow/cutover、operator、指标与发布门禁；
- W12 各 durable authority 的具体只读 adapter；
- 52 个 Migration Unit 的逐项 shadow、六门禁、单 owner 晋级、观察和 rollback；
- 生产 business schema migration、真实 Unit activation 及盘前/竞价/盘中/盘后验收。

因此 W09 完成的是“强终态证据不可伪造、最终化前必须二查”的权限边界，不等于生产推送改造完成，也不会改变当前线上推送。下一开发切片是 W10。
