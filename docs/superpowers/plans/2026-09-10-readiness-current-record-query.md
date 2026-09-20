# W15 同快照查询的当前记录消费接线

日期：2026-09-10。状态：本内部查询任务完成；初版 `c737110`、孤立分支修复 `14def95` 已提交，修复后8项query/12项store及格式/Clippy通过，独立初审质量Approved、限定复审问题关闭且无新阻断项，见[实施记录](../../push-system/implementation-readiness-current-query-2026-09-10.md)。初版相邻v3测试1项通过，本轮未改夹具、未重复运行；初版过程证据和activation直测Minor继续保留。此计划是完整 W15/W16 认证查询链的内部前置，不是以 candidate-only CLI 替代最终 health/readiness/CLI。现有 v3 集合、codec、恢复和存储已经交付，不重复实现。

权威要求：[W15 设计](../specs/2026-09-07-push-foundation-w15-operational-readiness-design.md) §3/6/7、[W15 计划](2026-09-07-push-foundation-w15-operational-readiness.md) Task4、[W16 计划](2026-09-08-push-foundation-w16-activation.md) T6，以及 [RFC 同快照查询合同](../../push-system/push-system-implementation-rfc.md#就绪查询与恢复合同-proposed)。该内部前置不得更改上述完整验收。

## 已核对的实施依据

- `readiness_store.rs::load_record` 在同一只读事务中核对完整链并允许返回当前 head 的历史祖先；它是合法历史查询，不能擅自改成仅当前。
- `load_head` 已核对当前链；缺少的是给定 snapshot ID 必须恰为当前 head 的单事务入口，不能让 caller 分开调用两个公开方法再自行拼接资格。
- `activation_readiness.rs::reread_activation_deployment_set` 已对全 catalog 重读并比较完整集合，拒绝配置、来源声明或任一 Unit 漂移；输入声明仍未认证。
- `readiness_deployment_set_tests.rs` 已有真实 activation → v3 record → recovery → store/reopen 夹具；复用它，不复制 codec、锁、完整性读取或采集算法。
- `StoredReadinessRecord` 明确只证明持久一致性，`readiness_probe` 仍为 caller-facts inventory；本任务不把两者改名成已认证权限。

## Global Constraints

- 只在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905`、`codex/push-reliability-20260905` 工作；不访问根工作树、真实 `.env`/`data/**`、生产 DB/monitor/provider/sink/订单/凭据、远端 Git 或部署。
- 单一实现者拥有下述 Rust 文件与测试；主线拥有计划、交付文档、证据、Git 和独立审查。测试仅使用显式临时目录、Test namespace 和合成的真实 SQLite 文件。
- 保留 `load_record/load_head/append` 原合同、v2 历史语义、v3/stream v2、legacy v1 拒绝、现有 canonical bytes/domain/SQL/schema/锁规则；不改 Cargo、冻结来源、蓝图制品或机器目录来掩盖源码漂移。
- caller 的 snapshot ID、路径、namespace、时间、部署声明都是选择/待核对输入，不能认证来源、owner/build 或授予 ReadyGate、发送及恢复许可。不新增公开 CLI、监控接线或 production-ready 输出。
- 跨库读取不是原子快照。限定结果只陈述本次读取检查成功，不能承诺返回后继续当前或执行安全；后续真实认证与执行临界区复验仍必需。

## Task 1: 当前 v3 持久记录的单一内部消费入口

目标：以后认证器从一个入口取得同一实际持久记录，不让 health/readiness/CLI 各自拼装候选状态。本次交付包含完整查询内部路径和真实存储反例；来源认证及最终消费者继续在总计划中实施。

文件 ownership：修改 `src/push_foundation/readiness_store.rs`、`readiness_store_tests.rs`、`mod.rs`；新增 `readiness_query.rs`、`readiness_query_tests.rs`。只有复用既有测试夹具所必需时，允许在 `readiness_deployment_set_tests.rs` 将现有 helper 可见性收窄为 `pub(super)`，不复制整套 fixture、不改既有测试断言；`activation_readiness_tests.rs` 已暴露的 helper 优先直接复用。不修改其他产品文件，范围需扩大时向主线说明理由。

1. 新增 `ReadinessRecordStore::load_current(snapshot_id)`，在**一个**既有受控只读事务中复用 `record_chain/head_chain`。必须验证完整记录链、当前 head 链及两者精确对应，只有所请求记录恰为 head 才返回 `StoredReadinessRecord`。合法但已被后续记录替代的 ID 返回明确 `HeadConflict`；不存在、孤立或损坏沿既有 fail-closed 错误，不自动修复、迁移或创建文件。旧 `load_record` 仍能读取合法历史。
2. 在新的内部 `readiness_query` module 提供一个 `load_current_v3_candidate` interface。它自行使用 bundled catalog，接收显式 readiness/activation 路径、namespace、snapshot ID、现有 activation reader 必需的 selected Unit 及 `ActivationDeploymentSetRequest`。可用一个小型借用请求结构表达参数，不新增通用 verifier/factory/插件或不必要的 wrapper。结果保留同一个 `StoredReadinessRecord`，不另造 snapshot，也不返回已认证类型。该 module 不在 `mod.rs` 公开重导出。
3. 顺序固定为：`load_current` → 明确拒绝 v2 → 从**已存记录**取出 v3 deployment set → 正式 `reread_activation_deployment_set` 核对全52Unit/配置/恢复覆盖/来源与共享声明 → 再次 `load_current` 并要求结果与首次记录精确相等。不得只比较 selected Unit、最大 generation、调用者 hash，或只调用初次 head 检查后直接返回。第二次读取是为发现 activation 重读期间 readiness head 变化，不声称多个数据库因此原子。
4. v2 只在新 v3 查询入口拒绝，不能降级回 v2 或破坏既有 v2 store/codec；未知/坏材料由正式 decoder 拒绝。namespace、catalog、完整集合不一致及数据库读取失败均返回脱敏 typed 错误，不把失败/缺记录当 Ready 或空候选。
5. 输出沿用现有 candidate 的安全 Debug；新增错误/请求 Debug 不暴露文件路径、protected URI 或证据正文。新入口不得调用 provider/sink、写连接、状态转换、初始化、恢复提交或通知；只读无副作用用真实库与目录字节/sidecar 检查证明，不用未接入的假计数器自证。
6. 先写行为测试并记录真实 RED，再最小实现。缺方法的编译失败单独记录，不冒充运行期缺陷。最低覆盖：当前 v3 重开后返回精确同一 record；合法历史被新入口拒绝但旧历史读取仍成功；新入口拒绝 v2；非 selected Unit 的代数变化、enabled/recovery 配置或跨库完整集合差异被拒绝；同次查询期间 head 被合法后继替代被拒绝；缺失/损坏存储、错误 namespace 和 protected URI 脱敏；成功和各只读拒绝路径不改实际两库/目录。竞态用仅 `cfg(test)` 的确定性 checkpoint，调用真实 SQLite 提交，不以 sleep 或 mock 返回值代替。测试名称和独立期望值应能抓住少掉第二次 head 检查、降为单 Unit 比较的退化。
7. 优先复用已有真实两 Unit/全52集合与 v3 记录 helper；如只调 helper 可见性，不重做旧 golden suite。迭代只跑当前新反例，最终一次运行 `cargo test --lib push_foundation::readiness_query_tests -- --test-threads=1` 与 `cargo test --lib push_foundation::readiness_store_tests -- --test-threads=1`；夹具可见性变化再定向运行受影响的既有 v3 store/reopen 测试。运行改动文件 rustfmt 检查、`cargo clippy --lib --message-format=json`，准确区分既有告警与目标诊断；不重跑全仓/全部 Foundation/Ruby/Chrome。
8. 实现者不做 Git 写入、不派子 agent；报告完整命令、实际输出/终态、RED/GREEN、文件清单和未验证项。主线固定 BASE..SOURCE 独立规格/质量审查，核对读路径、测试是否真实走新 interface、版本兼容与无权限升级，再交付中文结果。

完成边界：一个经过真实存储与漂移测试的内部 v3 当前记录入口；不是已认证 query/health/CLI，不是 W15/W16 完成。只要外部认证缺失，后续公开部署查询仍不得把此成功结果发布为生产 Ready。

## 后续完整链与回滚

此接口下一步由来源/部署认证器对**同一个 record**核验权威数据库身份、启用与恢复配置、business date/可信时间、source/acquisition/recovery 真实性和 owner/build，再供同一认证 handle 的 health/readiness/CLI 与 W11/W14 使用；不退回候选 CLI 或各消费者独立计算。实际信任根配置与生产批准不能从本地代码推定，仍按 W16 T2/T5/T6/T7 交付。

本任务修改 Rust 后，固定在 `aef7972` 的 current-source-audit/蓝图只是此前源码快照，不能称反映新 HEAD。主线在后续连贯实现批次完成时正式刷新当前审计及派生文档，不能放宽 freshness 或自动改冻结历史材料。完整 W01–W21/52 Unit 迁移、旧 v5–v9 兼容、实际 CI/生产接收等目标保持。

回滚仅反向撤销本任务新增入口/测试及对应声明，旧读取接口、持久数据、schema 和生产进程不动；不执行任何删除库或降级迁移。
