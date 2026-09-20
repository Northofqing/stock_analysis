# Push Foundation W09 实施计划

> 目标：以 TDD 落地只可由私有 authority 重查产生的 `VerifiedTerminalRef`，并强制 finalize 前二次重验；保持零生产接线。

## Task 1：冻结设计和 canonical 合同

- 固定 intent expectation、template binding、authority descriptor/record、错误分类与能力边界。
- 固定 `TemplateBinding/v1` 和 `TerminalBinding/v1` 的字段、枚举字符串、Option/嵌套值表示。
- 固定首次查询与 finalize 二查语义，明确 W10/W12 的后续接缝。

提交：`docs: design W09 terminal authority requery`

## Task 2：RED——完整 TerminalBinding 与首次重查

- 新增 W09 模块测试，先引用尚不存在的 authority port、record、template binding 和 verifier。
- 覆盖 golden binding、verified_at 排除、策略 authority、decision、rendered hash、subject 和其余业务字段变异。
- 覆盖 evidence bytes/hash、声明 binding hash、attempt/disposition 组合及 missing/pending/unavailable。
- 先运行精确测试，确认 RED 只来自 W09 缺失能力。

提交：`test: specify W09 terminal authority binding`

## Task 3：GREEN——attested expectation 与不透明引用

- W08 snapshot 增加 crate-private attested Ready binding 提取；每次提取重跑完整 integrity 校验。
- 完成策略增加 crate-private owner/authority 查询，不开放 caller-defined policy。
- 实现 template canonical binding、authority exact requery、全字段比较、evidence/binding 重算。
- 只有 verifier 可以把已校验 parts 送入 `VerifiedTerminalRef`；DeliveryResult 保持处置精确映射。

提交：`feat: verify authoritative terminal bindings`

## Task 4：RED——finalize 前二次重验

- 成功用例断言 authority 被调用两次且 verified_at 可变化。
- 在首次构造后变更 missing/pending/ref/disposition/evidence/业务绑定，逐类断言二查失败。
- 断言 prior 引用来自其他稳定 binding 时不能产生 finalization capability。

提交：`test: specify W09 finalization requery`

## Task 5：GREEN——fresh finalization capability

- 实现 `reverify_for_finalization`，强制重新查询、重复完整校验并比较 prior/fresh 稳定绑定。
- 只返回不可 Clone、crate-private 的 `FinalizationTerminalRef`，供 W10 按值消费。
- 不执行任何 business transition、cursor、schedule 或 durable 写入。

提交：`feat: require fresh terminal authority for finalization`

## Task 6：双轴 review 与修复

Standards：模块深度、权限最小化、错误脱敏、canonical 同源、无 raw DB、无 production constructor、测试 fixture 不泄漏。

Spec：RFC 全字段、authority/policy/schema、exact decision/bytes/subject、evidence/binding 重算、attempt 规则、人工接受映射、二次重查、compat 不可升级。

提交：`fix: align W09 authority verification with review`

## Task 7：Fresh 验证与中文结果

- W09 精确测试及 W01--W09 foundation/push_job 回归；
- rustdoc、`cargo check --lib`、strict/非致命 Clippy；
- 目标文件 rustfmt check（module root 使用 `skip_children=true`）；
- architecture docs 五组验证器、`git diff --check`、production-wiring relative diff；
- 只读 monitor PID/TCP/近期日志观测，不重启、不替换。

新增 `docs/push-system/implementation-w09-results-2026-09-07.md`，更新设计与 `.planning` 后提交。

提交：`docs: record W09 implementation evidence`
