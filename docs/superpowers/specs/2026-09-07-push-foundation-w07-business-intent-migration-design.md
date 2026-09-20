# 推送 Foundation W07 Business Intent Schema 与迁移设计

**状态：** 已实现并完成 fresh 验证；结果见 `docs/push-system/implementation-w07-results-2026-09-07.md`。未接 production database、caller、scheduler、provider、sink、finalizer 或 activation；W08--W21 与 52 个 Migration Unit 仍待完成。

**决策日期：** 2026-09-07

## 1. 目标和范围

W07 把 RFC 已冻结的 `docs/push-system/push-system-foundation.v1.sql` 落为显式、可审计、fail-closed 的迁移能力。它只负责：验证 DDL exact bytes、把 CLI 脚本应用到调用者明确给出的 SQLite 路径、拒绝不兼容入口，并在成功后只读证明 v1 schema signature 与登记对象闭合。

本切片不负责选择生产数据库，不在进程启动时自动迁移，不写业务 intent，不 claim lease，不追加 transition/outbox，不投递，不 finalizer，不恢复，不激活 Unit。W08 开始实现 append-only transition/outbox；W16 才组合 activation。W07 即使编译进制品，也没有生产行为。

## 2. 权威输入

- 唯一 DDL：`docs/push-system/push-system-foundation.v1.sql` exact bytes；SHA-256 固定为 `4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953`。
- schema version：`1`。
- description：`push-foundation-v1`。
- compatibility signature：`dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd`，身份是 `push-foundation-v1-final-wave1-not-delivered`，不是运行时计算的 DDL hash。
- managed inventory：25 个持久对象（六表、一个显式索引、十八 trigger）。
- WBS W07 验收：DDL 重复应用不改变已冻结原字节；版本/定义冲突拒绝；原库和非终态事实保留。

RFC 中嵌入的 SQL 只是 exact copy；独立 `.sql` 文件是唯一执行源。实现使用 `include_bytes!` 编译同一文件，先比对 SHA，再执行，不维护第二份 SQL 字符串。

## 3. 模块与接口

新增独立基础设施模块 `src/push_foundation/`，不把进程/SQLite 能力塞进纯领域模块 `monitor::push_job`：

```text
FoundationSchemaMigration::bundled()
  ├── 校验 embedded exact bytes SHA
  ├── 固定 schema version/signature/object count
  └── apply_to(&AbsoluteDatabasePath)
        ├── 直接 spawn /usr/bin/sqlite3（不经过 shell）
        ├── exact DDL bytes 写 stdin
        ├── .bail on + BEGIN IMMEDIATE 保证失败即停止/回滚
        └── read-only attest → MigrationReceipt
```

公开 API 只提供：

- `FoundationSchemaMigration::bundled()`：返回经 SHA 验证的 immutable capability；
- `apply_to(&Path)`：只接受明确绝对路径；不读取 env、不选择默认数据库；
- schema version、signature、DDL SHA getter；
- 成功返回 `MigrationReceipt`，包含 version、signature、DDL SHA、managed object count；
- typed `FoundationMigrationError`，不回显 SQL、数据库内容或 sqlite stderr 原文。

不提供 alternate DDL constructor、raw `Connection`、`execute_batch`、SQL override 或 production default path。

## 4. 路径与执行边界

`apply_to` 拒绝相对路径、缺少文件名、已存在目录/符号链接/非普通文件，以及不存在的父目录。这样调用者必须明确目标，不能因 cwd、symlink 或隐式目录创建把 migration 写到错误位置。

允许两种目标：已存在的普通 SQLite 文件，或位于已存在真实父目录下的全新文件。W07 不声称具有 durable-delivery 模块的 descriptor/OFD 强度；production path pinning、备份和操作窗口属于后续部署门禁。当前没有 production caller。

执行固定使用 `/usr/bin/sqlite3 <absolute-path>` 并通过 pipe 写 exact bytes；不使用 shell、不用字符串重定向，也不额外传 `-bail`。脚本首行 `.bail on` 是合同的一部分。spawn/write/wait/非零退出分别成为 typed error；stderr 只计算 SHA-256 供诊断关联，不进入错误正文。

## 5. 原子性和兼容拒绝

DDL 自身在 `PRAGMA` 后进入 `BEGIN IMMEDIATE`。任何 persistent CREATE 前先在 TEMP 表冻结入口 `sqlite_master`；只有以下两类入口允许继续：

1. 没有任何 managed object 的首次部署（无关独立表允许存在）；
2. 已有唯一正确 v1 schema 行、完整 25 对象登记，而且入口对象 name/type/definition exact bytes 与登记完全一致。

错误版本/签名、partial schema、缺对象、弱同名对象、额外挂接到 managed table 的 trigger/index、登记定义漂移均在补建前拒绝。失败保留既有独立表和业务行；不兼容库不自动“修复”、不 DROP、不覆盖、不注册为 v1。

重复应用正确 v1 只能经过兼容核验后执行 `IF NOT EXISTS`，不得改变：

- `sqlite_master.sql` 的已登记定义字节；
- `push_foundation_objects.definition` 字节；
- `push_intents` 已有身份、PreparedPush/rendered BLOB、hash、state/version/lease；
- `ResolutionRequired` 等非终态事实；
- 无关 legacy 表及数据。

这里的“原字节”指冻结对象定义和业务 payload BLOB，不声称 SQLite 容器文件每个 page byte 永远不变。

## 6. 成功后只读证明

CLI exit 0 后再以 read-only rusqlite 连接核对：

- `push_foundation_schema` 恰好一行，version/description/signature exact-match；
- `push_foundation_objects` 恰好 25 行；
- 每个登记项在 `sqlite_master` 存在、type 相同、`CAST(sql AS BLOB)` 与登记 definition 字节相同；
- 不存在挂在 managed tables 上但未登记的显式 index/trigger。

任何 post-apply mismatch 返回 `AttestationFailed`，不能因为 sqlite3 exit 0 就声称成功。重试同一 exact migration是安全恢复路径；API 不吞掉 ambiguous result。

## 7. 数据合同边界

本次应用完整冻结 schema，其中 `push_intents` 提前具备 Ready/NoData/Disabled、immutable payload、CAS 和 delete guard；`push_intent_transitions`、activation manifest/journal 也作为 additive Foundation 对象存在。但 W07 不提供这些表的写 API，也不把“表存在”冒充 W08/W10/W16 已实现。

W07 的测试可以直接插入受约束 fixture 以证明重复迁移保留事实；production 业务写必须等待后续 typed repository/transition/finalizer。

## 8. TDD 验收矩阵

1. embedded DDL exact SHA、首行 `.bail on`、version/signature/count 精确。
2. 相对路径、symlink、目录、不存在父目录 fail closed，且不 spawn migration。
3. 全新临时库应用成功；receipt 与库内 signature/25 对象登记一致。
4. 含独立 `push_legacy` 的库可增量建 Foundation，legacy definition/rows exact 保留。
5. 正确 v1 重复应用后，25 个 sqlite definition、登记 definition 和 Ready `PendingDispatch` 的 prepared/rendered bytes、hash、state/version exact 不变。
6. wrong version/signature、partial managed schema、缺失对象、定义漂移、额外挂表对象分别拒绝；入口 schema/legacy rows/非终态 facts 保留。
7. 非零退出错误只含 exit code/stderr SHA，不泄漏 stderr/SQL/业务 bytes。
8. 既有 W01--W06 测试、rustdoc、architecture-doc validators 保持通过。
9. `src/bin/monitor`、notification、durable_delivery、config、Cargo manifests relative diff 为零；生产 monitor 不重启、不替换。

## 9. 后续

- W08：intent repository、连续 version、previous hash、同事务 append-only transition/outbox 与崩溃恢复。
- W09--W12：authority revalidation、finalizer、reconciler、transport adapter。
- W13--W21：scheduler/readiness/activation/shadow/operator/metrics/gates。
- 52 Unit：逐一接线、shadow、六门禁、single-owner promotion、观察与 rollback。

W07 完成只证明兼容 schema 能被安全建立/重验，不证明 production DB 已迁移、业务 intent 已写入或任何消息已送达。
