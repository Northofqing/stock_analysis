# 推送 Foundation W06 Machine Catalog 运行时注册表设计

**状态：** 已实现并完成 fresh 验证；结果见 `docs/push-system/implementation-w06-results-2026-09-07.md`。未接 production caller、scheduler、provider、数据库、sink 或 activation；W07--W21 与 52 个 Migration Unit 仍待完成。

**决策日期：** 2026-09-07

## 1. 目标与权威边界

W06 把已经冻结的机器目录解析为只读、类型化、双向闭合的运行时注册表，关闭 `kind ↔ producer ↔ MigrationUnit ↔ completion owner` 的结构歧义。权威依据为：

- `docs/push-system/push-capability-catalog.v1.json` 的原始字节，SHA-256 固定为 `0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3`；
- `docs/push-system/push-system-implementation-rfc.md` 当前元数据的 65 kind、102 producer、52 Unit、状态计数与 enum 外 producer 范围；
- `docs/push-system/push-system-wbs.v1.json` W06 的验收：“65 kind/102 producer/52 Unit 双向闭合，枚举外入口保留独立注册”；
- `docs/Project_Architecture_Blueprint.md` / `.html` §24 的 Machine catalog、MigrationUnit 边界与 Foundation 零行为约束。

目录状态仍是 `PROVISIONAL`。`ACTIVE` 是当前事实分类，不是运行许可；W06 不读取或创建 activation generation，不产生 lease，不实例化 scheduler，不调用 producer，也不授予 physical ownership。W16 activation manifest 才能把已验证目录项与构建、generation、source/template 版本及 Disabled/Shadow/Active/Draining 状态组合。

## 2. 输入选择与非权威字段

实现使用 `include_bytes!` 把 machine catalog 的 exact bytes 编译进纯 library 模块；`bundled()` 先校验 exact SHA，再解析。这避免 cwd、部署文件缺失、运行时替换或 TOCTOU 造成目录漂移，同时没有文件 I/O。

W06 的 runtime authority 只消费机器目录中可机械闭合的字段：

- 顶层：schema version、status、baseline commit、enum evidence ID；
- kind：kind、primary phase、status、producer IDs；
- producer：ID、0/1 monitor kind、phase epics、occurrence family、completion owner、MigrationUnit ID；
- Unit：ID、completion owner、producer IDs、occurrence families、phase epics。

`trigger/source/authority/policy` 的自然语言描述、evidence IDs、known gaps 与 note 保持在审计文档，不进入运行授权。WBS 的依赖、估算、波次和 gate 是实施治理输入，也不因 W06 解析而成为 production activation。W06 不从叙述猜 source contract、template、schedule ID、audience 或目标 authority。

## 3. 类型化注册表

新增私有实现模块 `src/monitor/push_job/catalog.rs`，由 `push_job.rs` 暴露只读合同：

```text
exact embedded bytes + expected SHA
                │
                ▼
       MachineCatalog::bundled()
                │ validate before publish
                ├── 65 CatalogKindRegistration
                ├── 102 CatalogProducerRegistration
                │       ├── 92 Some(MonitorKind)
                │       └── 10 None / enum-external
                └── 52 CatalogUnitRegistration
```

值类型：

- `CatalogStatus = Active | Inactive | Starved | OptIn`，严格映射 `ACTIVE/INACTIVE/STARVED/OPT-IN`；
- phase 复用 W04 `PhaseEpic`，严格映射 `盘前/集合竞价/盘中/盘后`；
- ID 复用 W01 的 `ProducerId`、`UnitId`、`OccurrenceFamily`、`CompletionOwnerId`；
- kind 复用 W05 的 65 项 `MonitorKind`；
- baseline commit 复用 W04 `GitSha40`，catalog digest 复用 `Sha256Digest`。

所有 registration 字段私有并只提供 getter。`MachineCatalog` 自身构造完成后不可变，通过内部 `BTreeMap` 索引提供：

- `kind(MonitorKind)`；
- `producer(&ProducerId)`；
- `unit(&UnitId)`；
- `producers_for_kind(MonitorKind)`；
- `producers_for_unit(&UnitId)`；
- `unit_for_producer(&ProducerId)`；
- `enum_external_producers()`。

API 不提供 insert/remove/mutable getter，不提供 unknown-string fallback，也不把 note/description 当作执行参数。

## 4. 加载与发布顺序

`MachineCatalog::parse_v1_exact(bytes, expected_sha256)` 的顺序固定：

1. 对输入 exact bytes 求 SHA-256；不匹配立即拒绝，不尝试“容错解析”。
2. 严格反序列化所有结构 authority 字段；缺字段、错类型、重复已知字段或非法 JSON 拒绝。
3. 校验 `schema_version == 1`、`status == PROVISIONAL`、baseline Git SHA 与 enum evidence ID。
4. 校验 cardinality 恰为 65/102/52。
5. 把 kind/status/phase/ID/owner/family 转成闭集或受校验类型；未知值拒绝。
6. 在局部 builder 中建立索引并执行全部闭合检查。
7. 只有所有检查通过才返回不可变 `MachineCatalog`；不存在部分可用注册表。

`bundled()` 只调用这一条路径，并使用编译期常量 expected SHA。`parse_v1_exact` 只对 `push_job` 内部测试/版本实现可见，不是公开 authority；即便内部测试提供 mutation bytes 自身的 digest，也不能跳过 65/102/52、10 个 enum 外身份集合与闭合门禁。

## 5. 双向闭合不变量

### 5.1 kind 集合

- `MonitorKind::ALL` 的每一项在目录中恰好一行，无 missing/extra/duplicate；
- 状态计数严格为 36 Active、22 Inactive、5 Starved、2 OptIn；这是 RFC 当前冻结基线，不使用蓝图历史 37/24/2/2；
- 每个 kind 的 producer IDs 唯一且存在；producer 的反向 kind 必须完全一致；
- 无 producer 的 inactive kind 仍保留为明确注册，不能因空列表被删除。

### 5.2 producer 集合

- producer ID 恰好 102 个且唯一；每个 producer 只能绑定 0 或 1 个 MonitorKind；
- 92 个枚举内 producer 必须出现在相应 kind 的 producer_ids；
- 10 个 enum 外 producer 必须以 `monitor_kind=None` 独立保留并可查询，不能伪造第 66 个 kind，也不能折叠成 4 条概念路径；
- phase 列表非空、值合法且自身不重复；occurrence family、completion owner、Unit ID 均通过 typed ID 校验；
- 每个 producer 恰好属于一个存在的 Unit。

### 5.3 Unit 与 completion owner

- Unit ID 恰好 52 个且唯一；producer、occurrence family、phase 列表非空且各自无重复；
- Unit.producer_ids 与 102 producer 的 `migration_unit_id` 反向集合完全相等；
- Unit.completion_owner 必须与其每个 producer 的 completion owner exact bytes 相等；不 trim、不归一化、不按显示名合并；
- Unit.occurrence_families 等于成员 producer occurrence family 的去重集合；
- Unit.phase_epics 等于成员 producer phase 的并集；
- 两个不同 Unit 不得使用相同 completion owner。若共享 owner，应在同一原子 MigrationUnit，而不是两个可竞争 owner 的 Unit。

目录数组顺序不是 identity；关系比较使用集合。getter 保留目录声明顺序用于审计输出，但权限判断不依赖顺序。

## 6. 失败与日志安全

`MachineCatalogError` 只公开错误分类、实体类型、安全 ID、expected/actual count 或 SHA；JSON parse 错误只公开 line/column。错误不包含 note、trigger/source/authority/policy 描述、known gaps 或原始 JSON 片段。

需要独立覆盖：

- digest mismatch；
- unsupported schema/status；
- 65/102/52 数量漂移；
- unknown/duplicate/missing kind；
- duplicate producer/Unit；
- unknown phase/status；
- kind↔producer、producer↔Unit 反向不一致；
- completion owner、occurrence family、phase union 不一致；
- enum 外 producer 被删除、绑到假 kind 或在查询中丢失。

任何错误都必须 fail closed；不得打印输入 bytes 后返回“尽力可用”目录。

## 7. W04/W05 接缝

W06 注册表提供 `UnitId`、`ProducerId`、`OccurrenceFamily`、`CompletionOwnerId`、`MonitorKind` 和 phase 的受验证事实，供后续 composition 使用，但本切片不直接创建 `RunContextFactory` 或 `DecisionProjector`：

- machine catalog 目前没有 source-contract ID/version、template ID/version、audience、schedule ID 和 completion-policy version 的结构化字段；W06 不从自然语言描述伪造这些值；
- W16 activation/assembly 必须补齐这些 versioned binding，并验证 build/catalog SHA、generation 和状态后，才允许构造 W04/W05 运行能力；
- 在此之前，W04/W05 的非测试私有构造入口仍无 caller，保持 Foundation 零行为。

## 8. TDD 验收矩阵

1. bundled raw bytes SHA 与 RFC/WBS golden 完全一致，schema/status/baseline 可读。
2. 65 kind 精确覆盖 `MonitorKind::ALL`，状态计数为 36/22/5/2。
3. 102 producer、52 Unit 精确且所有 ID 唯一。
4. kind↔producer 和 producer↔Unit 两组关系双向完全闭合。
5. 每个 Unit 的 owner、occurrence family set、phase union 与成员 producer exact-match。
6. 92 enum 内 + 10 enum 外 producer 精确分栏；10 个 enum 外 ID 独立、可查询、均属于 Unit。
7. 代表项验证：`p01-scheduled → PreopenNewsHot → MU-p01`；`chain-preopen-timer → None → MU-chain-preopen`，owner/family/phase 同步。
8. digest、count、unknown kind/status/phase、反向引用、owner/family/phase drift 分别 fail closed。
9. malformed 输入错误 Debug 不包含插入的敏感描述或 JSON 正文。
10. 既有 W01--W05 golden/41 tests/rustdoc 保持通过。
11. relative production-wiring diff 继续为零；当前 monitor 不重启、不替换。

## 9. 非目标与后续

- W07：持久化 business intent/outbox，并把 PreparedPush immutable drift 转 ResolutionRequired。
- W08--W12：authority port、coordinator、专用 adapter、finalizer/reconciler 与恢复。
- W13--W16：scheduler/readiness、activation manifest 与经过验证的 assembly。
- W17--W21：shadow、operator、metrics、fault/backup/gate。
- 52 Migration Unit：逐 Unit shadow、六门禁、single-owner promotion、观察和 rollback。

W06 完成只证明目录结构可由运行时代码可靠解释，不证明任何 producer 可达、输入 ready、消息送达或 Unit 已晋级。
