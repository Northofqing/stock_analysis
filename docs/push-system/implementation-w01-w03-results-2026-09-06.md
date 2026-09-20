# W01--W03 推送合同内核实现结果

**实现分支：** `codex/push-reliability-20260905`

**比较基线：** `aad7ac16eef4007daedd62c100597bd10ea62e5b`

**验证日期：** 2026-09-07（Asia/Shanghai）

## 范围与非声明

本切片实现 `stock_analysis::monitor::push_job` 的三个纯合同：

1. W01：业务日、occurrence、source-contract 与 intent 稳定身份；
2. W02：应用层 `DeliveryResult`、强/弱 authority 隔离、当前 durable 十四态穷举投影；
3. W03：52 项 `ReasonCode`、有界 `RetryPolicy`、受控 `CompletionPolicy` 与四维完成判定。

本切片没有接入任何现有 producer、provider、LLM、sink、数据库、通知游标或 schedule owner；没有修改 `src/bin/monitor/**`、`src/notification/**`、`config/**`、`Cargo.toml`、`Cargo.lock` 或 `src/durable_delivery/**`。因此它不会改变当前实际推送的执行路径，也不表示 W04--W21、52 个 Migration Unit、shadow、cutover 或生产验收已经完成。

## 提交与文件

| 提交 | 内容 | 证据边界 |
| --- | --- | --- |
| `5d2b733` | W01--W03 设计 | 不透明结果、零接线与 W04+ 边界 |
| `dd0d8e4` | 逐测试实施计划 | TDD、验收命令与回滚边界 |
| `0a5054e` | W01 golden RED | 缺失 occurrence 合同导致预期失败 |
| `3adf2b3` | W01 GREEN | canonical-v1 与三层身份 |
| `2314549` | W02 GREEN | compatibility evidence、不透明结果与十四态投影 |
| `55a73fd` | W03 GREEN | 原因闭集、重试与完成策略 |
| `19735e8` | 双轴评审修复 | NoChannel authority 对齐、无 panic canonical encoder、证据补强 |

生产 seam 只有 [push_job.rs](../../src/monitor/push_job.rs)，实现按职责放在：

- [identity.rs](../../src/monitor/push_job/identity.rs)
- [delivery.rs](../../src/monitor/push_job/delivery.rs)
- [policy.rs](../../src/monitor/push_job/policy.rs)
- [tests.rs](../../src/monitor/push_job/tests.rs)

## W01 acceptance 证据

| 合同点 | 源码证据 | 测试证据 |
| --- | --- | --- |
| canonical preimage 为 ASCII domain + NUL + 有序 UTF-8 JSON | `canonical_preimage`、`write_json_object`、`write_json_string` | `w01_occurrence_golden_hash_is_stable` 同时断言精确 bytes 与 SHA；`w01_canonical_json_escaping_is_infallible_and_exact` 覆盖引号、反斜线、换行、制表符、中文和控制字符 |
| 原始 occurrence 只绑定业务日/family/key | `OccurrenceIdentityMaterial`、`derive_occurrence_id` | golden SHA 为 `5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a` |
| source-contract 防碰撞在外层身份实现 | `derive_schedule_occurrence_id`、`derive_intent_id` | `w01_outer_identities_bind_source_contract_without_changing_raw_occurrence` |
| Unit、producer、owner、namespace 不可误合并 | 两个外层 identity material 的私有字段 | `w01_outer_identities_bind_unit_producer_owner_and_namespace` |
| Test namespace 绑定 run id，subject 受校验 | `Namespace::Test`、`SubjectId::Entity` | `w01_test_namespace_and_subject_are_validated` |
| 日期、SHA、时间与文本值 fail closed | `BusinessDate::parse`、`Sha256Digest::parse`、`UtcMicros::try_new`、`validate_text` | `w01_identity_value_types_reject_invalid_input`，含空值、空白、NUL、非规范日期、非法 SHA、负时间与 513-byte 超长值 |
| 生产代码无序列化 panic | 领域专用不可失败 canonical encoder | `identity.rs` 生产区 `expect/unwrap/panic/todo` 扫描为空 |

运行：

```text
cargo test --lib monitor::push_job -- --nocapture
18 passed; 0 failed
```

## W02 acceptance 证据

| 合同点 | 源码证据 | 测试证据 |
| --- | --- | --- |
| 兼容证据渠道集合完整、唯一且 attempted 为 configured 子集 | `CompatibilityEvidenceRef::try_new` | `w02_compatibility_evidence_rejects_channel_shape_conflicts` |
| BestEffort/Partial/NoChannel/AllFailed 全部不得 finalization | 不透明 `DeliveryResult` 的受校验构造器与 `completion_eligibility` | `w02_compatibility_results_enforce_matrix_and_never_finalize` |
| `NoChannelConfigured` 属于 compat observation，不是 strong/none | `DeliveryResult::authority_class` | 双轴规格评审 RED 为 `left: None, right: Compat`，修复后同一 W02 测试通过 |
| 弱 `Unknown` 保持 Unknown，不自动重试 | `WeakOutcomeKind::Unknown` 与 `AllChannelsFailed` evidence | W02 matrix 测试直接读取 outcome，仍为 `Unknown` |
| 当前十四个 durable state 逐项投影且无 `_` | `classify_durable_state` 的穷举 match | `w02_all_fourteen_durable_states_have_exact_routes`，断言 case 数等于 14 |
| strong result 只能由 verified terminal 形成 | `VerifiedTerminalRef` 私有字段、无 public/Default/Deserialize/compat conversion | `w02_verified_terminal_disposition_controls_strong_result_permissions`；rustdoc `compile_fail` 1/1 通过 |
| manual accepted 不是 transport accepted | `from_verified_terminal` 的 disposition 路由 | manual accepted 只观察为 `DeliveryResultView::AlreadyTerminal` |

## W03 acceptance 证据

| 合同点 | 源码证据 | 测试证据 |
| --- | --- | --- |
| ReasonCode 是 RFC 精确 52 项、9 个 namespace 的闭集 | `reason_codes!` 生成无 wildcard 的 `ALL`、`as_str`、parse | `w03_reason_code_registry_is_exact_and_round_trips` |
| typed retry 保留原 reason，Uncertain 永不重发 | `RetryDirective`、`RetryPolicy::evaluate` | `w03_retry_directive_preserves_reason_and_never_retries_uncertain` |
| InputBackoff 不能被发送后或合同错误借用 | `ReasonCode::allows_input_backoff` 默认拒绝，仅列 5 个发送前输入原因 | `w03_input_backoff_cannot_retry_post_attempt_or_contract_failures`；修复前 RED 为 `EligibleInputRetry != Never` |
| Rejected 同时需要显式授权、attempt 未耗尽和到达 not-before | `RetryPolicy::AuthorizedRejected` | W03 retry 测试覆盖缺授权、未到时、次数耗尽、满足条件和错误 reason |
| NoData、Disabled、Uncertain 四维结果不同 | `CompletionFact`、`CompletionDirective`、`evaluate_completion` | `w03_no_data_disabled_and_uncertain_have_distinct_completion` |
| accepted/manual accepted/not-delivered/rejected 分离 | allowed authority、finalizer、cursor 与 `AdvanceEvent` 联合门禁 | `w03_strong_terminal_matrix_separates_schedule_cursor_retry_and_manual` |
| 四类 compat 只能使用 CompatibilityObservation | `require_compatibility_policy` | `w03_compatibility_results_require_observation_policy_and_never_complete` 覆盖 BestEffort、Partial、NoChannel、AllFailed |
| policy owner/cursor/authority/finalizer 组合 fail closed | `CompletionPolicy::register` | `w03_policy_registration_rejects_owner_cursor_authority_conflicts` 覆盖 owner mismatch、BoundCursor 无 cursor/authority、重复 authority、compat 携带 strong authority |
| 所有非终态事实不推进 cursor | completion matrix | `w03_non_terminal_facts_keep_cursor_closed_and_preserve_typed_retry` 覆盖 BlockedOnInput、Suppressed、RetryableFailure、PermanentFailure、Delivery::Blocked |

`evaluate_completion(policy, fact)` 按 RFC 没有 now、attempt count 或 durable retry authorization 输入，所以它对 Rejected 只能安全提出 `RejectedAuthorizationRequired`。调用方必须再把真实捕获事实交给 `RetryPolicy::evaluate`，不能在 completion evaluator 内虚构“当前已授权”。

## RED/GREEN 过程证据

- W01 golden RED 保存在提交 `0a5054e`。
- W02 各行为先以缺失 compatibility/result/十四态合同进入 RED，再逐项 GREEN；没有保留独立 W02 RED commit。
- W03 的 ReasonCode 首次 RED 为 `left: 7, right: 52`；completion 首次 RED 为缺失 `CompletionFact`/directive/policy fixture；InputBackoff 反例 RED 为 `EligibleInputRetry != Never`。这些 RED 实际运行，但 W03 没有保留独立 RED commit。

未保留 W02/W03 独立 RED commit 是相对实施计划提交粒度的过程偏差；本文不通过改写 Git 历史伪造它。功能验收由最终测试、源码 diff 与上述失败/修复记录证明。

## Fresh 验证命令与结果

| 命令 | 当前结果 | 解释 |
| --- | --- | --- |
| `rustfmt --edition 2021 --check <5 个 push_job Rust 文件>` | PASS | 本切片文件格式正确 |
| `cargo fmt --check` | BASELINE FAIL | 只报告未改动的 `src/bin/attribution_backfill.rs`、`src/bin/monitor/main.rs`、`src/bin/monitor/market_data.rs` |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS，18/18 | W01--W03 全部合同测试 |
| `cargo test --doc push_job` | PASS，1/1 | compat 无法转换为 `VerifiedTerminalRef` |
| `cargo check --lib` | PASS | 84 条既有 dead-code warning；局部标注后无 `push_job` 新 warning |
| `cargo clippy --lib --no-deps -- -A dead-code -D warnings` | BASELINE FAIL | 首个阻断为未改动 `src/data_gateway/futures_delivery.rs:15` 的 `empty_line_after_doc_comments` |
| `cargo clippy --lib --no-deps --message-format short -- -A dead-code` | PASS with 79 baseline warnings | 输出无 `src/monitor/push_job` warning |
| `cargo test --lib durable_delivery::tests -- --nocapture` | 104 pass / 12 fail | 失败集中于既有 BR192/BR206 并行 namespace nlink attestation；本切片对 durable 源码 diff 为零 |
| 两个 durable 代表失败 exact + `--test-threads=1` | PASS，2/2 | `decision_dedup_requires_identical_canonical_bytes` 与 `br192_manual_acceptance_is_revalidated_before_task_pending_reaches_delivered` 单独串行通过，支持并行互扰判断 |
| 五项 architecture docs Ruby tests | PASS，418 runs / 5023 assertions | RFC inputs、source catalog、catalog、RFC、WBS 全部零失败 |
| `git diff --check aad7ac1...HEAD` | PASS | 无 whitespace error |

实施前零源码变更基线的 `cargo test --lib` 已是 2816 passed / 32 failed / 7 ignored；至少一个 attribution 代表用例单独串行仍失败。因此本文不把全仓既有红灯归因于 W01--W03，也不冒充全仓测试全绿。

## 零生产接线证明

比较 `aad7ac1...HEAD`：

- 代码只新增/修改 `src/monitor/mod.rs` 与 `src/monitor/push_job*`；其余为设计、计划、结果文档；
- `git diff` 对 `src/bin/monitor`、`src/notification`、`config`、`Cargo.toml`、`Cargo.lock`、`src/durable_delivery` 均为空；
- `push_job` 依赖扫描没有 `rusqlite`、`diesel`、`reqwest`、gRPC、`std::env`、filesystem、provider、webhook 或发送调用；
- 没有 schema DDL、activation、owner cutover、游标写入或数据回填；
- `VerifiedTerminalRef` 真实 production 构造仍封闭到未来 W09，当前只有 `cfg(test)` fixture。

所以现有推送仍走原 release monitor 路径；本切片既不会中断它，也尚未提升它的生产可靠性。实际收益会在 W04--W21 和逐 Unit 接线后释放。

## 实际 monitor 隔离状态

开发期间只读复核仍为同一进程：

```text
PID 20162  Ss  ./target/release/monitor
```

2026-09-07 文档提交前最近一次已记录存活时长为 3:06:02。开发没有 rebuild、restart、stop 或 hot replace 该进程。此前运行观察仍显示本地 gRPC 18082 已连接、ExternalV1 `10.211.55.3:50051` 为 SYN_SENT；这属于实际运行 readiness 风险，但不授权本切片改变生产 owner。

## 双轴评审

### Standards

- 文档化仓库标准：`CLAUDE.md` 只给出 bounded context 与常用命令，没有额外编码门禁；`monitor::push_job` 位于 Market context。
- hard finding：0 个未解决。
- smell 判断：公共 seam 小而稳定；identity/delivery/policy 各自集中一种变化原因；值类型消除了 String/bool primitive obsession。`policy.rs` 较长，但它保持单一 completion-policy 职责，且公共构造面仍封闭，当前不构成 blocking divergent change。
- 工具基线：全仓 fmt/strict Clippy 的既有阻断已逐文件记录，未借机修改无关文件。

### Spec

- 评审发现 1：`NoChannelConfigured` 错标为 `DeliveryAuthority::None`，违反 RFC compat 行；已修为 Compat，仍保持 completion Never。
- 评审发现 2：canonical encoder 使用 production `expect`，违反设计“库代码不 panic”；已改成不可失败的领域编码器并增加 bytes/escaping golden。
- 未解决 blocking finding：0。
- 非阻断过程偏差：W02/W03 没有独立 RED commit；已透明记录，不改写历史。

结论：Standards 未解决 0 项，Spec 未解决 blocking 0 项；两轴最严重问题均已修复并由 18/18 模块测试覆盖。

## 已知限制与 W04--W21 / 52 Unit 剩余工作

W01--W03 只完成第一个 Foundation 切片。仍需按 WBS 依赖继续：

1. W04--W06：RunContext/PreparedFacts 单次捕获、纯 project、catalog owner/policy 注册；
2. W07--W10：intent/outbox、authority requery、TerminalBinding 与唯一 finalizer；
3. W11--W15：recovery、scheduler、readiness、activation、shadow；
4. W16--W21：operator、指标、测试 namespace、部署门禁和退出旧路径；
5. 52 个 Migration Unit：逐项 shadow、六门禁、单 owner cutover、观察、rollback 与证据归档；
6. 修复或隔离全仓既有 fmt、Clippy、durable 并行测试基线，使最终仓库级门禁真正全绿；
7. ExternalV1 readiness 恢复后再做生产运行验收，不能用 compatibility `pushed=1` 代替 durable receipt authority。

下一开发起点是 W04，且在 W09/W10 之前不得开放强终态 production 构造或推进通知游标。
