# Push Foundation W05 实施计划

> 目标：按 `2026-09-07-push-foundation-w05-projection-design.md` 以小提交、RED→GREEN 实现纯语义投影、七分支决定、PreparedPush 与首次 exact render bytes 封存；始终保持生产零接线。

## Task 1：冻结设计与门禁基线

- 新增并提交 W05 设计文档。
- 记录 W04 HEAD、目标测试、rustdoc、定向 rustfmt、architecture docs 与 production-wiring diff 基线。
- 验证现网 PID 只读存活，不重启、不替换。

提交：`docs: design W05 semantic projection and render seal`

## Task 2：合同 RED——语义类型与纯投影

在 `src/monitor/push_job/tests.rs` 先加入不能编译的目标测试：

- `MonitorKind::ALL` 精确 65 项、唯一、parse round-trip/unknown 失败；
- SubKind、Severity、Suppression 闭集和输入校验；
- 同一 W04 snapshot 重建 projection 的 canonical bytes/SHA golden 相同；
- source/model 顺序变化改变 evidence fingerprint；
- RunContext/facts SHA mismatch 拒绝；
- projection 14 字段 getter 和 context/template 强绑定。

运行目标测试并保存仅因 W05 symbol 缺失产生的 RED 证据。

提交：`test: specify W05 semantic projection contract`

## Task 3：合同 GREEN——SemanticProjection

- 新增 `src/monitor/push_job/projection.rs`。
- 实现 65 项 `MonitorKind`、`SubKind`、`Severity`、`Suppression`、`SemanticInput`。
- 实现 W06-private `ProjectionBinding` 与非 Clone `DecisionProjector`。
- 复用 canonical-v1，派生 EvidenceFingerprint 和 `SemanticProjection` exact canonical bytes/SHA。
- 在 `push_job.rs` 加入最小 re-export 和精确错误分支。
- 定向 rustfmt；运行 push_job tests，确保 W01--W04 golden 不变。

提交：`feat: add deterministic semantic projection`

## Task 4：合同 RED——PreparedPush 与 render seal

先加入不能编译的测试：

- Ready 11 字段、intent/decision/source binding/exact bytes golden；
- 有意空白保留、非 UTF-8 拒绝；
- 第二次 renderer 闭包不执行并计数；panic 后能力不重开；
- replay 反复读取首次 bytes，renderer count=1；
- same-intent immutable/render drift 返回 ResolutionRequired；
- 七个 JobDecision view；NoData/Ready/Suppressed 分支的事实和策略绑定；
- compile-fail 证明 render capability 不可 Clone。

运行目标测试并保存 W05 render/decision symbol 缺失 RED。

提交：`test: specify W05 prepared push and decision contract`

## Task 5：合同 GREEN——PreparedPush、JobDecision、单次渲染

- 实现 SourceBinding canonical、PreparedPush 11 字段与 stable DecisionId 派生。
- 实现 `ReadyPreparation` Open/Rendering/Sealed/Failed 状态、attempt/rejected 计数与 `FnOnce` renderer。
- UTF-8 只验证不归一化；Debug 不泄漏 rendered bytes。
- 实现不透明 JobDecision + JobDecisionView、七分支 canonical SHA 和约束构造器。
- 实现 same-intent immutable comparison，冲突只返回 `IntentPayloadConflict`/ResolutionRequired，不生成新 identity。
- 定向 rustfmt；运行目标测试与 rustdoc。

提交：`feat: seal prepared push and job decisions`

## Task 6：双轴 review 与修复

Standards 轴逐项检查：

- 深模块外部 seam 是否小且无 I/O；
- 是否复制已有类型、泄漏 bytes、过度 Clone、开放绕过构造；
- 是否加入 module-wide allow、全局时钟、serde panic 或自由字符串 authority。

Spec 轴逐项检查：

- SemanticProjection 14/14、PreparedPush 11/11、JobDecision 7/7；
- identity 排除 rendered/payload/evidence 摘要；
- same facts deterministic、首次 render sealed、replay no rerender；
- verified empty/source failure/suppressed/ready 不混淆；
- W05-only、zero production wiring。

先修复再复跑测试，以独立提交保留 review 证据。

提交：`fix: align W05 projection contracts with review`

## Task 7：验证、中文结果文档与收口

Fresh 执行：

- `cargo test --lib monitor::push_job -- --nocapture`
- `cargo test --doc push_job`
- `cargo check --lib`
- `cargo clippy --lib -- -A dead-code -D warnings`；若被目标外旧 lint 阻断，记录首个错误和精确归因，再运行非致命基线，不隐瞒；
- `rustfmt --edition 2021 --check` 仅列 W01--W05 文件；
- 5 个 architecture-docs 验证器；
- `git diff --check`；
- 对 `src/bin/monitor`、`src/notification`、`src/durable_delivery`、`config`、Cargo files 做 relative zero-wiring diff。

新增 `docs/push-system/implementation-w05-results-2026-09-07.md`，逐字段列实现行、测试、提交和例外；更新设计状态与 `.planning`。提交后验证 clean tree，再进入 W06。

提交：`docs: record W05 implementation evidence`
