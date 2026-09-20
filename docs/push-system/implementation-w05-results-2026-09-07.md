# 推送 Foundation W05 实现与验证结果

**结论：** W05 已实现并验证 `SemanticProjection`、七分支 `JobDecision`、`PreparedPush` 以及首次渲染字节封存合同。相同 `RunContext` 与同一个 `PreparedFactsSnapshot` 会得到确定的语义字节和 SHA；Ready 只允许一次 renderer 执行，后续重放只能读取第一次封存的 exact bytes。同一 intent 出现任何不可变 payload 漂移时必须进入 `ResolutionRequired(IntentPayloadConflict)`，不能换 identity 绕过冲突。

**边界：** 本文只关闭 W05 内存合同。W06--W21、52 个 Migration Unit、生产 catalog 装载、intent/outbox 持久化、authority/finalizer、shadow、physical owner 晋级及自然样本验收仍未完成。W05 没有接入当前生产 caller，所以不会改变现有推送行为，也不能证明某条现网消息已经送达。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 权威目标与实现边界

本切片逐项执行以下已冻结依据：

- `docs/push-system/push-system-implementation-rfc.md`：`SemanticProjection` 14 字段、`PreparedPush` 11 字段、`JobDecision` 七分支、`PreparedPushIntent/v1` 身份排除项与 exact rendered bytes 规则。
- `docs/push-system/push-system-wbs.v1.json` W05：同 facts 重建语义 hash 必须相同；首次渲染字节封存；重放不得重新渲染。
- `docs/Project_Architecture_Blueprint.md` / `.html`：Foundation 零生产行为变化；pure decision 与 presentation 分层；active/shadow 必须共用 W04 的单次事实。
- 设计：`docs/superpowers/specs/2026-09-07-push-foundation-w05-projection-design.md`。
- 计划：`docs/superpowers/plans/2026-09-07-push-foundation-w05-projection.md`。

W05 的生产边界刻意封闭：`ProjectionBinding` 构造器是 `pub(super)`，注释明确由 W06 machine catalog 成为第一个生产构造者（`projection.rs:203-240`）；`DecisionProjector` 没有 provider、模型、时钟、数据库、sink 或 scheduler 能力（264-292）。

## 2. 提交与 TDD 证据

| 提交 | 内容 | 证据性质 |
| --- | --- | --- |
| `4c5346d` | W05 语义投影与首次渲染封存设计 | 冻结范围、字段、状态机、失败边界；未接生产 |
| `f9359c5` | W05 逐测试实施计划 | 固定 RED→GREEN、双轴 review 和 fresh 门禁 |
| `cfaf987` | 语义投影合同 RED | 目标编译失败只来自 19 个 W05 缺失符号/合同 |
| `44252d8` | 确定性 `SemanticProjection` GREEN | 65 kind、闭集值、context 绑定和 canonical golden 落地 |
| `ce2a6c6` | PreparedPush/JobDecision 合同 RED | 目标编译失败只来自 36 个 W05 缺失符号/方法 |
| `d83b026` | 首次渲染、PreparedPush、七分支决定 GREEN | 11 字段封存、状态机、分支约束及 canonical SHA 落地 |
| `fbfeaf1` | 双轴 review 修复 | 构造边界、reason 分类、facts SHA、UTF-8 脱敏、identity 比较、测试强度闭环 |
| `83e1a4b` | Ready 内部存储压缩 | strict Clippy 发现的大 enum 修复为内部 `Box<PreparedPush>`；外部 view 不变 |

两个 RED 均早于对应实现提交；没有用语法错误、坏 fixture、网络、数据库或既有全库失败冒充目标失败。

## 3. 目录和值类型逐行证据

### 3.1 `MonitorKind` 精确 65 项闭集

- 宏同时生成 enum、`ALL: [Self; 65]`、稳定字符串投影和严格 `TryFrom<&str>`（`projection.rs:16-44`）。未知值只返回 `InvalidMonitorKind`（36-40），不能退化为自由字符串。
- 65 个 variant 在 `projection.rs:46-112` 逐项声明；集合覆盖 `HoldingEvent` 到 `NewsFlashAggregated`，没有运行时追加入口。
- `tests.rs:1519-1607` 独立写出预期 65 名，断言长度、顺序、唯一性、每项 parse round-trip，并断言 `UnknownKind` 被拒绝。

### 3.2 `SubKind`、`Severity`、`Suppression`

| 合同 | 实现行 | 不变量 | 测试 |
| --- | ---: | --- | ---: |
| `SubKindValue` | 114-125 | 构造私有；复用文本非空校验 | 1725-1727 |
| `SubKind` | 127-141 | 只允许 `None` 或 `Registered(validated)` | 1725-1727 |
| `Severity` | 143-162 | `Emergency/Important/Info/Research` 四项闭集；稳定字符串 | 1728 |
| `Suppression` | 164-184 | `Eligible` 或 reason + 可选 eligible_after | 1730-1738；reason 反例 2186-2198 |
| `SemanticInput` | 186-201 | 每次调用只允许 subject/severity/suppression 三项业务变量 | 投影与 suppressed 测试共同覆盖 |

目录字段和每次业务字段由类型边界分离：producer 不能借 `SemanticInput` 改写 audience、kind、owner、policy 或 template。

## 4. `SemanticProjection` 14 字段逐行证据

结构严格只有 RFC 的 12 个语义字段，加 `canonical_bytes` 和 `sha256` 两个封存字段（`projection.rs:432-448`）；字段全部私有，仅提供只读 getter（489-543）。

| # | RFC 字段 | 声明行 | 来源/构造行 | 测试证据 |
| ---: | --- | ---: | ---: | ---: |
| 1 | `audience` | 434 | catalog binding 453 | 1632 |
| 2 | `monitor_kind` | 435 | catalog binding 454 | 1633-1636 |
| 3 | `sub_kind` | 436 | catalog binding 455 | 1637 |
| 4 | `occurrence` | 437 | 冻结 projector 456 | 1638 |
| 5 | `business_subject` | 438 | typed input 457 | 1639-1642 |
| 6 | `severity` | 439 | typed input 458 | 1643 |
| 7 | `suppression` | 440 | typed input 459；reason 先校验 306-312 | 1644、2186-2198 |
| 8 | `completion_policy_id` | 441 | catalog binding 460 | 1645-1648 |
| 9 | `completion_policy_version` | 442 | catalog binding 461 | 1649 |
| 10 | `evidence_fingerprint` | 443 | 同一 facts 的 ordered refs 462、561-581 | 1652-1655、1667-1700 |
| 11 | `template_id` | 444 | catalog binding 463 | 1650 |
| 12 | `template_version` | 445 | RunContext 冻结值 464 | 1651 |
| 13 | `canonical_bytes` | 446 | `SemanticProjection/v1` exact preimage 466-469 | 1628-1631、1656-1659 |
| 14 | `sha256` | 447 | 直接由 exact canonical bytes 派生 470 | 1630-1631、1660-1663 |

`DecisionProjector::project_semantics` 在任何投影前核对 facts 的 `run_context_sha256`（298-305），再校验 suppression reason（306-312），最后调用纯构造（313）。跨 context 反例在 `tests.rs:1703-1721`，证明错误发生在 render 之前。

语义 canonical 输入精确位于 `projection.rs:583-629`；对象键由 W04 canonical encoder 排序，`SubKind` 和 `Suppression` 的 tagged object 分别在 631-642、644-665。`EvidenceFingerprint/v1` 对 `model_output_refs` 和 `source_refs` 各自保留采集顺序（561-581）；反转顺序会同时改变 fingerprint 和 projection SHA（`tests.rs:1667-1700`）。

确定性 golden：

```text
EvidenceFingerprint = be26edc2f3f2f21a8c2851de35d2935985b1742b329920e311d59e833274e606
SemanticProjection  = 7ea07990d419f36a9632fbff8dd52efbb383b51257166d57d9af3be5e7f058f1
OccurrenceId        = d5881448142d550c9e73bd4c7d61ed8da16587a6beee0f6dced16c5420b11e0d
```

完整 canonical preimage byte literal 固定在 `tests.rs:1656-1659`；同输入重建对象、bytes 和 SHA 三者相等（1622-1631）。

## 5. `PreparedPush` 11 字段逐行证据

结构严格只有 11 个私有字段（`projection.rs:702-715`），只读 getter 与 replay 入口在 753-799。

| # | RFC 字段 | 声明行 | 来源/构造行 | 测试证据 |
| ---: | --- | ---: | ---: | ---: |
| 1 | `intent_id` | 704 | W01 七项身份材料 724-732 | 1848-1863 |
| 2 | `decision_id` | 705 | 只由 intent 经 `PreparedPushDecision/v1` 派生 733、821-829 | 1864-1867 |
| 3 | `unit_id` | 706 | catalog-bound projector 738 | 1814 |
| 4 | `occurrence` | 707 | projector 739 | 1815 |
| 5 | `subject` | 708 | frozen semantic projection 740 | 1816 |
| 6 | `run_context_sha256` | 709 | projector 741 | 1817 |
| 7 | `prepared_facts_sha256` | 710 | W04 facts canonical SHA 742 | 1818-1821 |
| 8 | `semantic_projection_sha256` | 711 | projection SHA 743 | 1822-1825 |
| 9 | `source_binding` | 712 | facts contract/version/ordered refs + evidence fingerprint 744-747、668-700 | 1826-1838 |
| 10 | `rendered_bytes` | 713 | 第一次 renderer 原始 UTF-8 bytes 748 | 1839-1842 |
| 11 | `rendered_sha256` | 714 | 直接由 exact rendered bytes 派生 734、749 | 1843-1847 |

身份排除规则有代码证据：`derive_intent_id` 只接收 namespace、unit、completion owner、source contract ID、occurrence、subject、audience（724-732）；prepared-facts SHA、semantic SHA、source refs/evidence、rendered bytes/SHA 均不进入 identity。

golden：

```text
PreparedPush IntentId   = 1908f4826597b7203a9ae287078d76f52e4cd1be16a301c43eb91ff3043a66bb
PreparedPush DecisionId = d20eb0fd113dc403bdc2942e6ffe0b5626c2b53bffd731c0c33e5f6b1c9e854f
Rendered SHA-256        = d0bff0170bb997419bcc1c855032d1aba19c1befcdc17ed17e88c6accd552146
```

rendered golden 的输入是 `b"auction card  \nline two\n"`（`tests.rs:1799-1804`）；结尾换行和两个空格均保留，没有 trim、补换行或重编码。

## 6. 首次渲染封存与重放状态机

`ReadyPreparation` 不实现 Clone；rustdoc compile-fail 明确证明该能力不能复制（`projection.rs:847-855`）。其状态、尝试数和拒绝数是私有字段（855-862）。

```text
Open --render_once/FnOnce--> Rendering --valid UTF-8--> Sealed(PreparedPush)
                                  └--invalid UTF-8--> Failed
                                  └--panic----------> 保持 Rendering

非 Open 再调用：renderer 闭包执行前拒绝，rejected_count + 1
```

逐行行为：

1. Ready 创建前先重新绑定同一 facts/projection；verified empty 在 871-873 被拒绝，suppressed 在 874-876 被拒绝。
2. `render_once` 只接收 `FnOnce(&SemanticProjection) -> Vec<u8>`（899-902）。
3. 非 Open 在调用闭包前拒绝并计数（903-907），因此第二 renderer 不能产生副作用。
4. 闭包前状态切到 Rendering 且 attempt +1（908-910）；若闭包 panic，状态不会回到 Open。
5. UTF-8 只验证、不归一化；失败转 Failed，错误不携带正文（911-914）。
6. exact bytes 直接进入 `PreparedPush::from_first_render`（915-921），成功后才转 Sealed（922-925）。
7. 重放入口仅返回已存 bytes slice（797-799），没有 renderer 参数。

测试证据：

- `tests.rs:1792-1867`：renderer 恰好调用一次、状态/计数正确、11/11 字段及 exact bytes golden。
- `tests.rs:1871-1903`：第二闭包含 panic 但不执行；三次 replay 字节完全相同；总调用数仍为 1。
- `tests.rs:1906-1941`：非法 UTF-8 进入 Failed，Debug 不含 `rendered-secret`；renderer panic 后保持 Rendering，后续能力仍关闭。

## 7. 同 identity 漂移处理

`PreparedPush::compare_immutable` 在 `projection.rs:801-811` 只有三种结果：

- intent 不同：`DifferentIntent`；
- 11 字段完全相同：`Identical`；
- intent 相同但任一不可变字段不同：`ResolutionRequired { IntentPayloadConflict }`。

它不会为同 intent 的 payload/render 漂移生成新 decision ID。`tests.rs:1944-2025` 证明两份不同 rendered bytes 仍拥有相同 IntentId/DecisionId 并进入 ResolutionRequired；subject 改变造成不同 intent 时只返回 DifferentIntent，不误报漂移。

## 8. `JobDecision` 七分支及约束

内部 `JobDecisionKind` 精确七个 variant（`projection.rs:946-971`）；公开 `JobDecision` 是字段私有的不透明结构（973-981），调用者只能通过只读 `JobDecisionView` 观察相同七分支（1027-1051），不能自行伪造 Ready/NoData。每个分支以 `JobDecision/v1` + variant + typed payload canonical 化（1021-1023、1053 起）。

| 分支 | 构造约束实现 | 观察测试 |
| --- | ---: | ---: |
| `Ready(PreparedPush)` | 316-322、855-925；必须 non-empty 且 eligible | 2122-2126、2150-2175 |
| `NoData { reason, evidence_sha256 }` | 324-340；只允许 verified-empty + `IntentNoData`；evidence 是 PreparedFacts canonical SHA | 2030-2041、2135-2148 |
| `Disabled { reason }` | 342-353；只允许两个 disable reason | 2049-2057 |
| `BlockedOnInput { reason, retry_after }` | 355-369；只允许 input blocker | 2058-2070 |
| `Suppressed { reason, eligible_after }` | 371-389；必须来自冻结 projection 的 Suppressed | 2072-2101、2156-2198 |
| `RetryableFailure { reason, retry_after }` | 391-405；reason 必须允许 input backoff | 2102-2111 |
| `PermanentFailure { reason }` | 407-419；只允许永久 preparation failure | 2112-2120、2200-2210 |

七个实际 decision 的 canonical SHA 必须两两不同，测试使用集合插入验证 7/7 唯一（`tests.rs:2122-2131`），不再使用“自己等于自己”的弱断言。`TransportUncertain` 既不能冒充 suppression，也不能在发送前冒充 permanent preparation failure（2186-2210）。

## 9. 双轴 review 与修复结果

### 9.1 Standards 轴

| 发现 | 风险 | 修复及证据 |
| --- | --- | --- |
| projection/catalog 构造入口过宽 | W05 可绕过 W06 machine catalog | `ProjectionBinding` 与 projector 构造收回 `pub(super)`；生产首构造者明确留给 W06 |
| public enum 可伪造 Ready/NoData | 跳过 facts/policy 约束 | 改为不透明 `JobDecision` + 只读 view + 受校验构造路径 |
| rendered bytes 可能被错误规范化 | 重试 payload 改变 | exact `Vec<u8>` 只做 UTF-8 验证；空格/换行 golden 固定 |
| UTF-8 错误可能泄漏正文 | 日志暴露消息/敏感内容 | 错误无 bytes 字段；测试断言 Debug 不含 fixture secret |
| Ready 使 enum 触发 `large_enum_variant` | 栈尺寸和 strict Clippy 门禁新增错误 | 仅内部改为 `Ready(Box<PreparedPush>)`；外部 `JobDecisionView::Ready(&PreparedPush)` 不变；strict 错误数 80→79 |

### 9.2 Spec 轴

| 发现 | 风险 | 修复及证据 |
| --- | --- | --- |
| reason 只靠类型、未按分支分类 | transport/policy/input reason 可串分支 | `project_semantics` 和六个 non-ready 构造器逐分支校验；反例覆盖 |
| NoData evidence 可能间接断言 | 无法证明绑定 exact PreparedFacts | 测试直接先取 `empty.facts().canonical_sha256()` 再精确比较 |
| decision hash 测试过弱 | canonical variant/payload 碰撞不被发现 | 七个真实分支 SHA 放入 `BTreeSet`，断言 7 个唯一值 |
| 同 identity 与不同 identity 混淆 | 正常不同 subject 可能被当作漂移 | comparison 先比 intent；测试同时覆盖 same-intent drift 与 different-intent |

review 后没有遗留 W05 critical/high/medium finding。内部 `Box` 是实现尺寸修复，不改变 RFC 对外字段、身份或 canonical 表达。

## 10. Fresh 验证结果

| 命令/门禁 | 结果 |
| --- | --- |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS：41 passed / 0 failed / 2855 filtered；catch_unwind 的 panic 输出是预期反例，测试均为 ok |
| `cargo test --doc push_job` | PASS：3 passed / 0 failed / 17 filtered；terminal authority、capture、Ready render capability 三个 compile-fail |
| `cargo check --lib` | PASS；84 条目标外既有 dead-code warning；无 W05 编译错误 |
| `cargo clippy --lib -- -A dead-code -D warnings` | 基线阻断：修复 W05 `large_enum_variant` 后从 80 降为 79 个目标外旧 lint；首个仍为未修改 `src/data_gateway/futures_delivery.rs:15`；无 `push_job` error |
| `cargo clippy --lib -- -A dead-code` | PASS，exit 0；79 个 warning 均不在 `push_job` |
| 定向 `rustfmt --edition 2021 --check` | PASS：9 个 W01--W05 文件 |
| RFC inputs validator | PASS：12 runs / 238 assertions |
| source catalog validator | PASS：12 runs / 110 assertions |
| catalog validator | PASS：33 runs / 393 assertions |
| RFC spec validator | PASS：295 runs / 3873 assertions |
| WBS validator | PASS：66 runs / 409 assertions |
| architecture docs 合计 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| `git diff --check 989afa7..HEAD` | PASS |
| production-wiring relative diff | PASS：`src/bin/monitor`、`src/notification`、`src/durable_delivery`、`config`、`Cargo.toml`、`Cargo.lock` 均无 diff |

strict Clippy 的 79 个错误均为本切片外既有 lint。本文不把它写成全仓全绿，也不越权批量修改目标外模块。

## 11. 生产 monitor 只读观察

2026-09-07 收口时的最后一次只读检查：

```text
PID 20162, etime 05:00:33, state Ss, RSS 206404, command ./target/release/monitor
10.211.55.2:60076 -> 10.211.55.3:50051 ESTABLISHED
127.0.0.1:54144 -> 127.0.0.1:18082 ESTABLISHED
```

该进程仍是 W05 开发前已启动的同一 release monitor；没有重启、重建、热替换或加载本 worktree 代码。TCP `ESTABLISHED` 只证明观察时连接存在，不证明某条消息被 transport accepted，更不等于 durable receipt、用户可见送达或业务完成。

## 12. 零行为变化与剩余工作

相对 W04 结果提交 `989afa7`，W05 只修改文档和 `src/monitor/push_job*` 合同文件。以下生产路径 relative diff 为零：

```text
src/bin/monitor
src/notification
src/durable_delivery
config
Cargo.toml
Cargo.lock
```

因此开发过程中现有推送仍走旧生产路径，不受 W05 影响；同时也意味着 W05 尚未把可靠性合同应用到任何具体盘前、集合竞价、盘中或盘后 Unit。

下一步 W06 必须用 machine catalog 成为 `ProjectionBinding` 的唯一生产创建入口，并校验 Unit、65 kind、owner、policy、template、source contract 与 phase/trigger 的 exact match。之后仍要完成 W07--W21，以及 52 个 Migration Unit 各自的 shadow、六门禁、single-owner 晋级、观察和 rollback 证据，才构成整个推送改造完成。
