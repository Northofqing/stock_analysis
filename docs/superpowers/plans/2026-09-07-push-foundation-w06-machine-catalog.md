# Push Foundation W06 实施计划

> 目标：按 W06 设计，以小提交和 RED→GREEN 实现 exact-byte、类型化、双向闭合的 machine catalog 运行时注册表；始终保持零生产接线。

## Task 1：冻结设计与基线

- 提交 W06 设计；记录权威 catalog SHA、65/102/52、36/22/5/2、92 enum 内与 10 enum 外基线。
- 核对 WBS W06 scope/acceptance 及蓝图 Foundation 边界。
- 只读确认生产 monitor PID，不重启、不替换。

提交：`docs: design W06 machine catalog registry`

## Task 2：合同 RED——bundled catalog 与只读查询

先在 `src/monitor/push_job/tests.rs` 加入不能编译的目标测试：

- bundled SHA/schema/status/baseline；
- 65 kind 与状态计数；
- 102 producer、52 Unit；
- 92 enum 内、10 enum 外；
- p01 与 enum 外 chain 代表项的 kind/unit/owner/family/phase；
- public getter/query 无字符串 fallback。

运行目标测试，保存只因 W06 symbol 缺失产生的 RED 证据。

提交：`test: specify W06 machine catalog contract`

## Task 3：合同 GREEN——解析、typed entries 与索引

- 新增 `src/monitor/push_job/catalog.rs`；`include_bytes!` 固定 machine catalog。
- 实现 `CatalogStatus`、三个 registration 和 `MachineCatalog` 只读 API。
- 先校验 exact SHA，再解析 schema/status/typed fields/counts。
- 使用局部 builder 构造 BTreeMap 索引，成功后一次性发布不可变对象。
- 在 `push_job.rs` 加入最小 module/re-export；不加入 caller。

提交：`feat: add typed machine catalog registry`

## Task 4：合同 RED——闭合与 fail-closed 反例

在成功基线上逐项 mutation，并用 mutation bytes 自身 digest 排除“只被 SHA 拦住”的弱测试：

- missing/duplicate/unknown kind；
- producer/Unit count 和 duplicate；
- kind↔producer、producer↔Unit 反向 drift；
- completion owner mismatch/duplicate owner；
- occurrence family set 与 phase union mismatch；
- enum 外 producer 删除或错误绑定；
- unknown status/phase；malformed JSON 与错误脱敏。

先运行并保存精确失败，再实现验证器。

提交：`test: specify W06 catalog closure failures`

## Task 5：合同 GREEN——双向闭合验证

- 实现全部 set equality、uniqueness、owner grouping 和 enum-external 验证。
- 错误只携带分类、安全 ID/count/SHA/line/column。
- 不容错修补、不跳过坏行、不发布 partial registry。
- 运行 W06 目标测试及全 push_job tests。

提交：`feat: enforce machine catalog closure`

## Task 6：双轴 review 与修复

Standards：检查模块 seam、错误脱敏、无 I/O/全局可变状态、无自由字符串 authority、无过宽 constructor、无无关依赖/allow。

Spec：逐项核对 65/102/52、36/22/5/2、92+10、两组双向关系、owner/family/phase closure、PROVISIONAL 非 activation 和 zero wiring。

先修复再复跑，以独立提交保留 review 证据。

提交：`fix: align W06 catalog registry with review`

## Task 7：Fresh 验证与中文结果

- `cargo test --lib monitor::push_job -- --nocapture`
- `cargo test --doc push_job`
- `cargo check --lib`
- strict/非致命 Clippy，精确记录目标外基线；
- 定向 `rustfmt --edition 2021 --check` W01--W06 文件；
- architecture-docs 5 个验证器；
- `git diff --check` 和 production-wiring relative diff；
- 只读 monitor PID/TCP 检查。

新增 `docs/push-system/implementation-w06-results-2026-09-07.md`，逐关系列实现行、测试、提交和例外；更新设计状态与 `.planning`，提交后确认 clean tree。

提交：`docs: record W06 implementation evidence`
