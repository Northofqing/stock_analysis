# Push Foundation W07 实施计划

> 目标：以两轮 RED→GREEN 落地 exact DDL capability、显式 SQLite CLI migration 与 post-apply attestation；保持零生产接线。

## Task 1：冻结设计和执行边界

- 提交 W07 设计；固定 DDL SHA、schema signature、25-object inventory 和 WBS 验收语义。
- 明确 `.bail on` CLI script 不能传给 `execute_batch`。
- 明确绝对显式路径、无默认 production DB、无自动启动迁移。

提交：`docs: design W07 business intent migration`

## Task 2：合同 RED——元数据、路径和 fresh apply

先增加目标测试，引用尚不存在的：

- `FoundationSchemaMigration::bundled()` 元数据；
- path validation；
- fresh SQLite apply 与 `MigrationReceipt`；
- unrelated legacy table 保留。

运行目标测试，保存只由 W07 symbol 缺失产生的 RED。

提交：`test: specify W07 foundation migration contract`

## Task 3：GREEN——exact script runner 与只读 attestation

- 新增 `src/push_foundation/mod.rs` 与 `migration.rs`；在 `lib.rs` 只增加 module export。
- exact SHA 校验，固定 `/usr/bin/sqlite3`，stdin 写入 exact bytes；不经过 shell。
- typed path/spawn/write/exit/attestation error，不回显 stderr 原文。
- read-only 核对 schema row、25 个登记对象、definition bytes 和清单外挂接对象。
- 不增加 caller、DB default、startup hook 或 Cargo dependency。

提交：`feat: add exact foundation schema migrator`

## Task 4：合同 RED——重复、冲突与事实保留

在临时 SQLite 文件中覆盖：

- 正确 v1 二次应用保留 definitions 和 Ready/PendingDispatch exact blobs/state/version；
- 独立 legacy 表/row 保留；
- wrong schema/signature、partial managed object、missing object、extra attached trigger/index、definition drift；
- 每个拒绝前后读取并比较原事实，证明没有事后补建/覆盖；
- error Debug 不泄漏 sqlite stderr/业务 payload。

提交：`test: specify W07 compatibility rejection`

## Task 5：GREEN——补齐 fail-closed 分类和恢复语义

- 仅增加满足反例所需的最小 validation/attestation。
- 所有非零 CLI 结果返回 typed rejection；不解析自然语言决定兼容性。
- 成功必须带 attested receipt；失败不能返回 partial success。

提交：`feat: enforce foundation migration compatibility`

## Task 6：双轴 review 与修复

Standards：模块边界、进程生命周期、path 安全、错误脱敏、无 raw connection 泄漏、无 SQL 复制、无 production default。

Spec：逐项核对 exact SHA、`.bail on`、首次 additive、25-object exact registry、重复不变、冲突不修复、nonterminal/legacy 保留和 zero wiring。

提交：`fix: align W07 migration with review`

## Task 7：Fresh 验证与中文结果

- W07 目标测试与全部 push foundation/push_job tests；
- rustdoc、`cargo check --lib`、strict/非致命 Clippy；
- 定向 rustfmt check；
- architecture docs 五组验证器；
- diff-check、production-wiring relative diff；
- 只读 monitor PID/TCP 检查。

新增 `docs/push-system/implementation-w07-results-2026-09-07.md`，更新设计与 `.planning` 后提交。

提交：`docs: record W07 implementation evidence`
