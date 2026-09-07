# W15 运行就绪快照与恢复事件设计

**状态：** 候选判级、snapshot/event codec、目录计数、独立 schema 与候选原子 store 已实现并通过限定范围独立评审。真实来源认证、权威 probe 和 W11/W14 联结尚未实现，当前不代表完整 W15 或生产就绪已接入；最新验证见 W15 中文实现记录。

## 1. 目标与已有约束

实现 WBS W15 的三范围判级和版本化恢复关联。权威规格是 `push-system-implementation-rfc.md:993`--`:1047`，启动顺序依据蓝图 §24.15。独立就绪判定、持久事实和只读探针组合成一个可测试模块；就绪事实不替代消费数据时的 freshness/identity 检查，也不授予 W16 的 physical-owner/fence 权限。

已核实的代码：

- W06 `MachineCatalog` 登记 65 kind、102 producer、52 Unit，10 个 producer 位于枚举外；Inactive kind 可以没有 producer。
- W11 `StartupRecoveryReport` 是成功恢复遍历的结果，W14 使用其中的 marker；marker 不是跨库、跨 Unit 的全局许可。
- W14 保留原 occurrence 与有效延期窗口，输出纯转换提案；ReadyGate 和 input recovery 尚待本阶段联结。
- `grpc_source.rs:1428`、`:1626` 通过 BR-159 `record_data_acquisition` 保存每路诊断审计，返回值/日志不是完整 OperationalReadinessSnapshot。
- `main.rs:3834`、`:5527` 的静态/实时诊断已作为独立后台任务运行。W15 不重新引入“单路数据失败阻断所有 resident loops”。

## 2. 存储选择

采用显式路径、独立 schema version 的 operational SQLite 数据库，管理 readiness evidence；不向业务 intent/receipt 库混入新的通知完成状态，也不修改 W07 v1 的 25 对象冻结 SQL。写入侧在隔离开发数据上显式初始化，查询侧使用 read-only/query-only 打开，查询不得创建数据库或自动迁移。

取舍：仅使用内存不能证明重启后可查询；把就绪状态塞进日志缺少 exact snapshot/recovery join；修改冻结 W07 SQL 会破坏既有签名。独立 operational 数据库保留这三个已有合同，并让 health、readiness、CLI 共同查询。

新增对象使用 `operational_readiness_` 前缀，范围限于 schema header、immutable snapshot、immutable recovery event 和 current-head。snapshot/event 有内容哈希与唯一键，禁止更新/删除；只有 head 能在 `BEGIN IMMEDIATE` 中 CAS 更新。建立路径、schema 和 namespace 身份验证，复用现有安全路径检查的可用部分，不静默接受未知或部分 schema。

新库不保存 prepared/rendered/receipt/source 正文，不形成另一套投递 authority。producer 接线、生产路径和初始化操作留到获准的部署流程。

只读文件副作用边界补充：operational v1 使用 rollback-journal 文件格式；WAL/未知版本在 SQLite 查询前拒绝，不自动转换、checkpoint 或用 immutable=1 忽略未合并事实。原因是 SQLite 的 READ_ONLY WAL 打开仍可能在可写目录创建 `-wal`/`-shm`（[官方 WAL §5](https://www.sqlite.org/wal.html#read_only_databases)）；[文件头读写版本](https://www.sqlite.org/fileformat.html#file_format_version_numbers) 可在查询前识别。初始化必须原子取得缺失目标的创建所有权，防止竞争空文件被接管。上述零副作用/文件所有权要求已在 `64047dd` / `3fe1e62` 后通过对应测试与独立限定复审，不产生来源认证权限。

第二轮接口裁决（已实现并验证）：首次初始化不再让 SQLite 按目标路径写 DDL，而是在内存库建立、验证原 schema/header，再把完整序列化镜像经 `create_new` 返回的同一个 `File` 写入并同步。文件身份由实际持有句柄保证，不以内容哈希替代；若路径在写入期间漂移，不能接管替换者。仅增加 rusqlite 的 `serialize` feature（本地 0.31 实现依赖 modern_sqlite/bundled_bindings，无新增 crate，也不强制 bundled SQLite）；实际库链接与运行已通过 Cargo 测试，不以命令行 sqlite3 版本替代证据。

existing reader/writer 的 interface 收窄为受控事务操作，不向调用者返回可长期持有的裸 Connection。内部通过 SQLite 公开的 [sqlite3_file 方法](https://www.sqlite.org/c3ref/io_methods.html) 从实际句柄取得 SHARED lock 并读文件头，在进入可能产生副作用的 schema 查询前验证 rollback 格式；锁必须连续覆盖到同一连接的受控事务，不能在检查与读取间释放。新连接强制 PRIVATECACHE，并在 schema prepare 前设置和验证 [main.locking_mode=EXCLUSIVE](https://www.sqlite.org/pragma.html#pragma_locking_mode)，使首次 schema prepare 内部读事务结束后仍保留读锁；不能仅凭 raw guard 对象存活假定锁连续。SQL 前拒绝可释放原始锁，进入 SQL 后不提前 xUnlock，由事务结束和连接关闭管理。读取 callback 只在事务中运行，写入 callback 使用 IMMEDIATE 事务；安全生命周期、失败释放和错误脱敏集中在这一 module。未来 store 只使用此 seam，不重新打开路径、结束事务或改变 journal/query_only。可以用窄私有 helper 管理公开 FFI，禁止猜测 unixFile 内存布局、改全局 VFS、在可变原库使用 immutable/nolock 或用独立普通文件锁替代 SQLite 锁。

代价：原 schema/open 返回 Connection 的内部接口和测试需调整，增加公开 SQLite FFI 的生命周期验证及 serialize 链接验证；尚无生产接线或其他生产消费者，不改变用户推送语义。若任一步不能证明零副作用和锁连续性，应失败关闭并报告，不能把“查完才发现变化”当作预防写入。schema/open 仍不产生来源认证或恢复许可。

第三轮锁规则修订（`3fe1e62` 已通过实测与限定复审）：existing 主库文件头统一通过实际 sqlite3_file 的 xRead 读取，不额外使用普通 File::open/read/close。普通 close 可能取消同进程其他 SQLite 连接的 POSIX 锁，因此“双重检查”会破坏持锁保证；测试在连接存活时也不能直接读取主库字节，并须用独立进程竞争验证。保留路径/存在性、NOFOLLOW/PRIVATECACHE、EXCLUSIVE 和全部未知文件拒绝规则。修复前本机反例也通过，不能声称本机复现；关闭依据是危险路径移除及收紧后的独立进程断言通过。此修订不重开已关闭的初始化 owned-file finding，也不授权全局 VFS 替换或生产操作。

## 3. 模块与小接口

文件规划：

- `src/push_foundation/operational_readiness.rs`：类型、注册校验、三范围判级和纯 assessment。
- `src/push_foundation/readiness_snapshot.rs`：候选快照的规范化身份、完整证据引用绑定与 Debug 脱敏；单独成文件以免把编码细节混入分类器。候选值不是 attested snapshot。
- `src/push_foundation/readiness_snapshot_codec.rs`：从持久化 canonical bytes 重建候选材料并重新判级，核对完整重新编码字节；不能仅相信行内 status 或由调用者重算的哈希。
- `src/push_foundation/readiness_store.rs`：schema、证据复验、snapshot/recovery 原子追加、只读重查。
- `src/push_foundation/readiness_store_schema.rs`：独立 SQLite 的 schema/header/namespace 与显式初始化、只读/写入打开；不提供认证或投递权限。
- `src/push_foundation/readiness_recovery.rs`：无环 recovery identity 与 before/after snapshot 材料；认证来源仍由 store 的实际 reader 复验，候选事件不是恢复许可。
- `src/push_foundation/readiness_recovery_codec.rs`：从事件正文与两端候选快照重建 identity、实际依赖差异及恢复声明，逐字节核对全部关联；不认证外部来源。
- `src/push_foundation/readiness_probe.rs`：来自同一 snapshot 的 health/deployment/CLI 投影。
- `src/bin/push_readiness_probe.rs`：显式路径的只读命令；不启动 monitor、不进行 provider 调用。
- 各模块对应 `_tests.rs`；内部模块仅在 `push_foundation/mod.rs` 注册，最终仅导出 probe 必需的只读接口。

工作链：

```text
catalog + 当前注册/部署合同 + 已核验的依赖证据
  → ReadinessAssessment（纯判级结果，不是执行许可）
  → store 复验来源、版本、前一 snapshot 与提交前提
  → 同事务追加 recovery event + snapshot + head CAS
  → AttestedReadinessSnapshot
  → health / deploy probe / CLI / W14 控制面判定
```

纯 assessment 的构造和测试不能铸造 attested snapshot；后者只能由存储提交后的精确重查生成。类型命名保持这一区别，避免把可构造的 bool/diagnostic report 提升成权威。

纯判级保留实际 scope、catalog SHA、启用 producer、依赖声明和正/负观察，按稳定键排序；Ready 时受影响集合虽为空，也不能丢掉被评估范围。明确失败观察保留原 ReasonCode 和证据哈希，拒绝把成功、NoData 或无关投递结果登记成依赖不可用。候选快照在编码前要求每条观察与 evidence ref 的依赖角色、来源、版本、哈希一一匹配；缺失/重复/多余引用拒绝。此检查只证明内部绑定一致，不证明外部来源真实或 recovery 已发生，认证与 event 前后 join 仍由存储负责。

## 4. scope 与依赖合同

`ReadinessScope` 为 Core、Producer、Occurrence。Producer 包含 W06 精确 producer/Unit 绑定；Occurrence 还绑定已存在的业务 occurrence ID。所有提交外层绑定 namespace、authority business_date、build、manifest SHA、generation、catalog SHA 和 capture time。

每个依赖声明稳定的 `DependencyKind`、`SourceContractId`、`SourceContractVersion` 及预期 authority。观察必须指向明确依赖、实际版本和证据引用，不能只提供可自报的 Ready。缺观察、重复观察、未知依赖、版本或来源漂移必须可区分；重复/额外输入拒绝请求，依赖缺失或不匹配保留 typed 失败原因并判不就绪。

| 类别 | 必需角色 | 缺失影响 |
| --- | --- | --- |
| Core | Namespace、Durable、Audit、TypedAuthority、Schema、Manifest | 所有启用 producer/Unit 的 CoreUnready |
| Producer | ProducerBinding、SourceContract、ScheduleOrTrigger、Presentation、DurablePolicy、ReceiptStrength、FeatureGate、CompletionPolicy | 对应 ProducerUnready |
| Occurrence | 已完整注册的 Producer 合同 + OccurrenceInput | 仅输入证据缺失时为 BlockedOnInput；合同缺失仍为 ProducerUnready |

声明由显式 typed 注册提供，不能从 W06 自然语言解析。暂不适用的依赖也必须有带版本和依据的明确 NotRequired 声明，不能通过漏掉角色来得到 Ready。Core/Producer 的 required 角色闭集由 evaluator 检查。

启用 producer 集合只描述待评估范围，不是 activation 授权。它必须是 W06 已知 producer 的去重集合；受影响 Unit/producer 集合由 evaluator 推导，不信任调用方手填。无 producer 的 Inactive 元数据不创建执行项；Starved/OptIn 的原批准边界保留，评估不能自动启用它们。

## 5. 判级与退出

| 状态 | reason | liveness | deployment_ready | exit disposition |
| --- | --- | --- | --- | --- |
| Ready | activation.ready | true | true | Continue |
| CoreUnready/启动 | activation.core_unready | false | false | StartupNonzero |
| CoreUnready/运行中 | activation.core_unready | true，直到受控退出 | false | StopNewAndRecoverIsolateThenNonzero |
| ProducerUnready | activation.producer_unready | true | false | IsolateAffectedContinueOthers |
| BlockedOnInput | input.source_unavailable | true | true | ContinueWithoutOccurrenceWork |

最终 deployment projection 对同一当前上下文内的 scopes 聚合；任何 Core/ProducerUnready 都不能被其他 Ready/Blocked 覆盖。ProducerUnready 同时输出可查询的 operational alert 事实，发送仍交给既有治理；模块不新增通知旁路。

Readiness 只控制是否可以考虑新工作；消费者仍复验当次数据与 W16 当前 fence。Ready 不代表收到股票数据、不代表发送成功，也不意味着可以推进通知游标。

## 6. 快照、证据与恢复

Snapshot 实现 RFC 全部字段；canonical hash 使用已有 `CanonicalValue`/`canonical_digest` 规则并排除自身 ID。依赖/受影响集合按稳定键排序且拒绝重复，输入 evidence 的角色与版本精确绑定。查询必须核验 schema、canonical bytes/hash、head join 和恢复链，不能只 SELECT status。

重建候选材料时先核验 domain/字节哈希，再解析闭集字段、用当前精确 catalog 重新执行 assessment 和 evidence 绑定，最后要求重新编码字节完全相同。这样即使篡改者同时重算 status 所在行的哈希，也不能把缺依赖事实改写成 Ready；重复字段、额外字段、非规范序列和派生状态漂移同样不能被解析器静默丢弃。此流程仍不认证外部 evidence 或 recovery，store 的只读复验与 join 不可省略。

每个 evidence ref 包含类型、protected URI、SHA、source ID/version。Debug/错误/probe 只输出稳定 ID、类型、版本、hash，不暴露 URI 正文或内容。注册 authority 通过现有 immutable audit/本地 authority 验证结果复验引用；字符串 actor 或自报 success 不能签发恢复。实现时只开放经验证 reader 的 attestation 构造，测试替身限制在测试 seam，不产生生产授权。

实际 reader 接线裁决：现有 BR-159 acquisition audit 只能证明完整持久原始事实；其记录缺 namespace、authority business_date、SourceContractId/version 和 W15 scope/部署上下文，`schema_version=1` 不能代替合同版本，按 capability+provider 派生的 previous_outcome 也不能独立证明特定 scope 恢复。先在 database 模块复用现有全链/receipt 算法，接受显式现有连接、同一 DEFERRED 事务返回私有构造的原始审计事实，不接受调用者自称的 expected observation，不通过全局 DatabaseManager 初始化取得连接。此入口没有 opener 或来源身份认证；最终必须由受信 source descriptor 与 typed 版本化解释将原始 outcome/时间等绑定到 W15，不能把 receipt SHA 或任意 URI 文件当来源证明。`verified_empty` 保留原义，不直接推导依赖不可用或业务 NoData 完成。

KnownOccurrence 后续从真实持久 intent 与 transition chain 的同一只读快照取得，再与 catalog 的 producer/Unit/family/owner 精确关联；须包含 Ready、NoData、Disabled 等合法持久 intent，不用仅支持 Ready 的 binding 接口或 RunContext 派生 ID 替代存在性。其余 Core/Producer 角色及版本化 NotRequired 仍需真实 typed 注册/部署 authority，不用 terminal accepted 或 schema receipt 兜底认证。

首次记录追加 ReadyObserved 或 Pending 事件；恢复须有显式且经认证的 capability/version 变化，重新评估通过后追加对应 CoreDependenciesRestored / ProducerContractRestored / InputEvidenceRestored。只有时间流逝、重复 tick、空 Vec 或未经验证的日志变化不构成恢复。

为避免循环 hash：先根据前 snapshot ID、context、依赖变更和认证来源材料派生 recovery_event_id；snapshot 将该 ID 纳入 hash；event 正文最后绑定 before/after snapshot hash 并另存完整 event SHA。重查同时验证 event identity、正文 SHA 和两个 snapshot 引用，不允许借“ID 不包含 after hash”偷换 after snapshot。

同一事务写 event、snapshot、head；head 版本经 checked add。重复提交读取同一事实；陈旧 head、输入漂移、写失败整体回滚。提交确认未知先重查，不能盲目追加。通过重新打开真实临时 SQLite 验证跨重启行为。

提交确认异常的实现边界：仅 COMMIT 阶段异常在原事务及连接结束后触发完整 `load_record(snapshot_id)` 重查；候选全部材料精确相同才返回原持久 receipt。无记录、损坏、查询失败或材料漂移都只能报告“未确认”，不能把错误解释成未落库，也不能自动再次追加。成功快路径和明确的提交前错误不额外查询。验证须区分真实 SQLite COMMIT 拒绝、成功提交后的确认丢失模拟与尚未覆盖的底层 I/O 故障，不能相互冒充。

持久集成切片的 StoredReadinessRecord 仅包含已重建的 CandidateReadinessRecord 和已核验链版本，不构造 AttestedReadinessSnapshot。stream 身份与 recovery continuity 一致，包含 namespace、业务日、build、generation、manifest、catalog、scope 和启用 producer 集合；capture time、依赖观察和 stage 不作为新 stream。查询迭代重建完整前序链，拒绝循环、断链、跨 stream 和 head 版本漂移。外部来源认证仍须在权威发布前完成，不能把数据库落库与字节一致误称为证据来源可信。

## 7. probe 与计数

只读 probe 返回 snapshot hash、build、generation、affected IDs、reason、recovery event、exit disposition，以及明确单位的计数。错误/过期上下文/缺失/损坏数据失败关闭，不能默认为 Ready；probe 不采集新数据、不写 snapshot、不触发恢复。

默认 kind 视图的 `push_total=65`，包含 Inactive 元数据；就绪需检查该 kind 的实际已登记 producer，不能因空 producer 集合全称判断而变 Ready。Starved/OptIn 按 conditional 单独呈现，Inactive 数量来自 kind registration；compat 和 missing 类计数按 typed 绑定事实去重，不解析描述文字。另提供 producer 总数 102、枚举外 producer 总数 10 及其独立就绪/故障集合，确保 CLI/产业链路径不被 kind 视图遗漏。各 missing 指标可重叠，不能加总冒充总数。

health、readiness 与 CLI 共享一个已验证 snapshot 的投影函数；探针本身没有独立权限缓存。W16 的上下文绑定和 production 接线仍须在提交/执行临界区重验。

## 8. W11/W14 联结

W11 恢复遍历成功后才可评估新工作的 readiness。W14 的时间窗口判定与 readiness assessment 保持不同职责，orchestration 只使用已经重查的匹配范围快照。

非就绪 Core/Producer 阻断对应新 occurrence/prepare；其他就绪 producer 继续。OccurrenceInput 缺失可提案 Expected/Eligible→BlockedOnInput；经认证的原 occurrence 恢复事件加有效窗口/ReadyGate，才可提案 BlockedOnInput→Eligible。窗口已结束按 W14 policy 处理，Closed/Missed 无出边，延期继续使用保留的有效窗口。

上述仍是纯提案；实际状态与转换证据的业务库事务、当前 activation/fence 属于 W16/逐 Unit 实现。W15 不把 readiness store 当成 occurrence storage owner。

## 9. 可检验交付顺序

1. 三范围缺依赖/错版本分类、受影响集合与退出策略：先通过纯 assessment seam 写行为反例。
2. Canonical snapshot/recovery 的无环身份、证据复验、原子存储及重启查询。
3. 同一 snapshot 的 probe/CLI/计数和只读零副作用证明。
4. W11/W14 联结和恢复/窗口/版本反例。
5. Spec/Standards 评审、相邻回归与中文结果文档。

完成 W15 必须有真实隔离存储和 probe 证据；只有第一条分类器通过时不称 W15 完成。完整目标仍包括 W16--W21 与 52 Unit，生产批准、晋级和交易日观察按原合同保留。
