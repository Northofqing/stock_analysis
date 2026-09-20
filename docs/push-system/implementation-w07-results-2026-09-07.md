# 推送 Foundation W07 实现与验证结果

**结论：** W07 已实现并验证 additive Business Intent schema 迁移能力。代码只接受调用者明确给出的绝对 SQLite 路径，只执行仓库冻结的 exact CLI DDL；首次应用建立并证明 v1/25-object schema，正确 v1 重复应用保持对象定义、PreparedPush/rendered BLOB 和非终态行不变，不兼容版本、partial/missing/extra object 均在自动修补前拒绝并保留原事实。

**生产边界：** 没有选择、打开或迁移任何生产业务库；没有 startup hook、monitor caller、默认 DB path、intent 写入、transition/outbox、authority、finalizer、scheduler、sink 或 activation。W08--W21 与 52 个 Migration Unit 仍未完成。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 权威材料

| 项目 | 冻结值 | 代码证据 |
| --- | --- | --- |
| 唯一 DDL | `docs/push-system/push-system-foundation.v1.sql` exact bytes | `migration.rs:11` 使用 `include_bytes!`，没有第二份 SQL |
| DDL SHA-256 | `4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953` | `migration.rs:12`；fresh `shasum` exact-match |
| schema | version 1 / `push-foundation-v1` | `migration.rs:13-14` |
| compatibility signature | `dd5f49a1…60ecdd` | `migration.rs:15`；是固定版本身份，不冒充 DDL 内容 hash |
| inventory | 25 个对象 | `migration.rs:16`；六表、一个显式 index、十八 trigger |
| executor | `/usr/bin/sqlite3` | `migration.rs:17`，符合 RFC 的 CLI script 接口 |

DDL 自身首行 `.bail on`、第 9 行 `BEGIN IMMEDIATE`、第 375 行 `COMMIT`。核心对象分别位于 SQL 55/62/82/190/240/290 行；登记与 schema signature 写入位于 364/371 行。独立 SQL 文件没有被本切片修改。

## 2. 提交与 TDD 证据

| 提交 | 内容 | 证据性质 |
| --- | --- | --- |
| `6d0a2be` | W07 设计与逐测试计划 | 冻结 exact authority、路径、事务、兼容与零接线边界 |
| `385bb41` | 第一组合同 RED | 唯一 `E0432`：迁移 capability/error 尚不存在 |
| `fd574c3` | exact migrator GREEN | 4/4：metadata、path、fresh apply、legacy 保留 |
| `528c840` | 第二组兼容 RED | 唯一 `E0599`：父目录 symlink 拒绝类型尚不存在 |
| `9110af2` | compatibility GREEN | 9/9：二次应用与各类冲突/保留合同通过 |
| `228f939` | 双轴 review 修复 | 去除生产 panic、错误分类、query-only readback、拒绝诊断 SHA 加固 |
| `9e0cbe7` | module 格式稳定 | 新 module 声明按 rustfmt 稳定排序；不改行为 |

两轮 RED 都只由预期 W07 symbol/variant 缺失导致，对应 GREEN 在后；没有用 SQL 环境失败或全仓旧失败冒充红灯。

## 3. 小而封闭的公开接口

`push_foundation` 是独立基础设施模块（`src/lib.rs:58`），没有污染纯领域模块 `monitor::push_job`。`mod.rs` 只导出：

- `FoundationSchemaMigration`；
- `MigrationReceipt`；
- `FoundationMigrationError`。

`FoundationSchemaMigration` 只有私有 DDL digest 字段（`migration.rs:62-64`）；公开构造只能是 `bundled()`（67-74），先重算 embedded exact bytes SHA，再发布 capability。schema/description/signature/SHA/count getter 位于 76-94；原始 DDL getter 只在测试编译中存在（96-99）。没有 alternate DDL constructor、SQL override、raw connection 或默认 production path。

成功 receipt 的四个字段均私有，只能读取 version、signature、DDL SHA 和对象数（`migration.rs:138-161`）。它证明本次 post-apply attestation 通过，不证明业务数据、回执或 Unit 已激活。

## 4. 路径 fail-closed

`validate_database_path` 位于 `migration.rs:163-196`：

1. 拒绝相对路径和无文件名目标；
2. 父目录必须存在、可读、为真实目录且自身不是 symlink；
3. 已存在目标必须是普通文件，不能是 symlink、目录或不可读元数据；
4. 允许在已存在真实父目录内创建全新文件。

`tests.rs:29-59` 覆盖相对路径、缺父目录、目录目标、目标 symlink；`tests.rs:440-453` 证明父目录 symlink 在 sqlite3 启动前拒绝，真实目录内没有生成 DB。

本能力没有 durable-delivery 的 descriptor/OFD 强度，也没有 production path pinning/backup；因此仍未接 production。这个限制是明确边界，不被 receipt 隐藏。

## 5. exact CLI 执行与错误安全

`apply_to` 位于 `migration.rs:101-134`：

- 直接 `Command::new("/usr/bin/sqlite3")`，数据库路径作为单独参数，不经过 shell；
- exact embedded bytes 通过 child stdin `write_all`；不 trim、不重编码、不额外拼 `-bail`；
- stdout 丢弃，stderr 仅在失败时计算 SHA-256；
- spawn、stdin write、wait、non-zero exit 分别成为 typed error；
- 非零退出返回 exit code + stderr SHA，不保留/回显 stderr、SQL 或业务正文；
- exit 0 仍不能直接成功，必须继续 post-apply attestation。

错误闭集在 `migration.rs:20-59`。review 删除了 digest helper 的 `unreachable!`，生产路径没有 `unwrap/expect/panic`。错误版本测试使用带 `migration-secret` 的路径和 legacy 内容，Debug 均不含这些值，同时断言 exit 非零、stderr SHA 为 64 位（`tests.rs:257-330`）。

## 6. 成功后的只读证明

`attest` 位于 `migration.rs:198-273`：

1. 使用 `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`；
2. 设置并读回 `PRAGMA query_only=1`（202-216）；
3. schema 表必须恰好一行且 version/description/signature exact-match（218-232）；
4. registry 必须恰好 25 行（234-243）；
5. 25 个 registry 项逐一与 `sqlite_master` name/type/definition BLOB exact-match（245-254）；
6. managed tables 下不得出现未登记的显式 trigger/index（256-265）；
7. 所有检查完成后才构造 receipt（267-272）。

因此 `sqlite3` 进程的成功退出不是唯一成功依据；无法只读复核会返回 `AttestationOpenFailed/AttestationFailed`。

## 7. 重复应用与事实保留

### 7.1 正确 v1 重复应用

`tests.rs:146-173` 分别以 BLOB 读取 registry definition 和 live `sqlite_master.sql`。`tests.rs:175-214` 插入一条合法 Ready/PendingDispatch fixture，PreparedPush 与 rendered bytes 都包含 NUL、尾部空格或换行。

`tests.rs:217-255` 二次执行同一 DDL 后证明：

- registry 25 项 definition bytes 全部不变；
- live 25 项 sqlite definition bytes 全部不变；
- prepared/rendered exact BLOB 不变；
- state 仍为 `PendingDispatch`；
- version 与 lease_generation 仍为 0。

这里证明的是冻结定义/业务 BLOB 的“原字节”，不虚构 SQLite 容器文件每个 page byte 永不变化。

### 7.2 无关 legacy 数据

`tests.rs:101-143` 先建立独立 `push_legacy` 并写入含 NUL 的 BLOB；首次 Foundation apply 后，其 CREATE definition 与 row bytes exact 不变。符合 additive migration，不清理、不纳管无关表。

## 8. 不兼容入口拒绝

| 反例 | 测试 | 拒绝后保留证据 |
| --- | ---: | --- |
| wrong version/signature | 257-330 | 原 schema definition、version/description、legacy BLOB 不变；未补建 metadata |
| partial `push_intents` | 332-383 | 原弱表 definition/BLOB 不变；未补建 schema/registry |
| 正确 v1 缺 recovery index | 385-438 | 重跑拒绝，index 仍缺失，不静默修复；PendingDispatch exact bytes/state/version 不变 |
| 正确 v1 外挂 rogue trigger | 385-438 | 重跑拒绝，rogue object 仍保留供处置；PendingDispatch exact facts 不变 |

兼容性由冻结 CLI DDL 的入口快照/transaction/trigger 决定；Rust 不解析 stderr 自然语言来猜“可修复”。失败返回单一 typed rejection，原库留给另行批准的迁移处理。

## 9. 双轴 review 结果

### Standards

- 将 SQLite/进程能力放在独立 `push_foundation`，保持 `push_job` 纯合同。
- API 不读取 env/cwd 默认 DB，不泄漏 raw connection/SQL override。
- 路径错误细分 missing/unreadable/not-directory/symlink；不把权限错误误报 missing。
- child 总是 wait；stderr 不进入 Display/Debug，只保存 digest。
- post-apply connection 强制并读回 query-only。
- 删除生产 `unreachable!`，digest invariant 也走 typed error。

### Spec

- exact DDL/SHA、`.bail on`、fixed signature、25-object inventory 全部绑定；
- fresh/additive/repeat/legacy/nonterminal preservation 都有行为测试；
- wrong/partial/missing/extra 均拒绝且证明不修复；
- 完整 schema 中 transition/activation 表存在，但 W07 没有写 API，不冒充 W08/W16 完成；
- 无 production database/caller/activation 接线。

review 后没有遗留 W07 critical/high/medium finding。

## 10. Fresh 验证

| 命令/门禁 | 结果 |
| --- | --- |
| W07 target suite | PASS：9 passed / 0 failed / 2906 filtered |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS：51 passed / 0 failed / 2864 filtered；两条 panic 文本是既有 catch_unwind 反例，测试均 ok |
| `cargo test --doc push_job` | PASS：3 passed / 0 failed / 17 filtered |
| `cargo check --lib` | PASS；84 个目标外既有 dead-code warning；无 W07 error |
| strict Clippy | 基线阻断：恰好 79 个目标外旧 lint；首个仍为 `src/data_gateway/futures_delivery.rs:15`；无 `push_foundation` error |
| nonfatal Clippy | PASS，exit 0；79 warnings；无 W07 warning |
| 定向 rustfmt | PASS：14 个 W01--W07 文件，使用 `skip_children=true` 隔离目标外旧格式 |
| architecture docs | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| `git diff --check e992585..HEAD` | PASS |
| production-wiring relative diff | PASS：monitor/notification/durable_delivery/config/Cargo manifests/frozen SQL 均无变更 |

strict Clippy 的 79 项是既有目标外基线，本文不把它写成全仓 strict lint 全绿。最初不带 `skip_children` 的 rustfmt 检查递归显示目标外旧格式；没有自动修改它们，改用逐文件 `skip_children=true` 后目标范围通过。

## 11. 生产 monitor 与下一步

验证期间只读观察：既有 `./target/release/monitor` 仍为 PID 20162，运行约 6 小时，两条既有 TCP established。没有重启、重建或热替换。连接存在不能证明消息被接收，更不能替代 typed Accepted/AlreadyDelivered 回执。

下一步 W08 实现 business intent repository、连续 version、previous hash、同事务 append-only transition/outbox 与逐崩溃边界恢复。此后还需 W09--W21 和 52 个 Unit，整体项目才可能进入生产晋级阶段。
