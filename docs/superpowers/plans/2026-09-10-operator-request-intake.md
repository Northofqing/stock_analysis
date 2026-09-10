# W18 操作员请求解析与精确身份绑定

日期：2026-09-10。状态：准备实施，尚未派发实现。此任务补齐操作员控制面的实际请求入口，不是完整 W18，也不替代 W15/W16 的认证、执行和生产验收。

## 依据与当前事实

权威为 [RFC 操作员请求与输出](../../push-system/push-system-implementation-rfc.md#操作员请求与输出-proposed)、[操作员命令](../../push-system/push-system-implementation-rfc.md#操作员命令-proposed)、[操作员权限](../../push-system/push-system-implementation-rfc.md#操作员权限-proposed)，以及 [W16 计划 T2/T5/T7](2026-09-08-push-foundation-w16-activation.md)。这些合同同时要求完整应用执行、独立审计和真实认证，不能用本任务的输入校验替代。

2026-09-10 在提交 e22c881 的未改相关源码中核对：

- `src/monitor/push_job/context.rs` 的 `CommandId` 与 `AuthenticatedOperatorRef` 是普通受限文本，不是 SHA-256 强类型或身份认证结果。现有 manual trigger 使用它们；不能为新 wire 改坏旧合同，也不能直接将其构造成功当新 wire 验证。
- `src/monitor/push_job/identity.rs` 已有严格小写 64 hex 的 `Sha256Digest`、非负且受 i64 限制的 `UtcMicros`、`Namespace` 等；canonical-v1 的 domain + NUL + 排序 JSON 实现在 `canonical.rs`，应复用。
- `src/push_foundation/activation_authorization.rs` 已有 `ApprovalBinding` 和原始批准声明比较，但没有操作员 JSON 请求入口；声明比较不认证 issuer，不持久防重放。
- `business_finalizer.rs::VerifiedOperatorAuditRef` 仍仅在 `cfg(test)` 下可构造。不能在解析请求时签发这个 capability。
- 现有 `ReasonCode` 是闭集，未知值的底层解析错误可能携带原文本；新 wire 必须映射为脱敏错误，不直接回显完整输入。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 的 `codex/push-reliability-20260905` 开发；不访问根工作树、真实 `.env`/`data/**`、生产 DB/monitor/provider/sink/订单/PAM/凭据、远端 Git 或部署。
- 当前 W15 query 实现/验证先完成，再串行交接共用 `mod.rs`；不得与其同时修改产品文件或启动并行 Cargo。
- 本入口是未认证请求解析，不签发 Ready、部署批准、当前执行许可或 operator audit capability；不增加自由构造的 `Verified` 类型或任意 verifier 插件。
- 保留既有 manual `CommandId` 文本合同、W15 v2/v3、冻结 SQL、历史 canonical domain/bytes 和八份冻结文档。不改 Cargo，不用全仓格式化，不通过改目录或 hash 掩盖源码漂移。
- 主线拥有计划、交付记录、Git 与独立审查；实现者只拥有 Task 1 列明的代码/测试，不做 Git 写入、不派子 agent。测试只用内存值；没有真实认证/生产副作用的入口，不以假的外部调用计数宣称生产安全。

## Task 1: 严格请求入口与命令绑定

### 目标和 interface

新增内部 `src/push_foundation/operator_request.rs`、`operator_request_tests.rs`；仅在 `mod.rs` 注册 module 和 test module，不公开重导出。提供一个从有界 JSON 字节到 `UnverifiedOperatorRequest` 的入口及只读 getter/canonical bytes/request digest。调用者不自行拼接命令字段、选择规范化规则或把 claimed identity 升为已认证身份。

完整接收 RFC 十字段：`command_id`、`command`、`target`、`expected_version`、`expected_generation`、`dry_run`、`authenticated_operator_ref`、`reason`、`evidence_refs`、`requested_at`。所有字段必须出现；没有省略 `dry_run` 时自动 apply 的默认值。拒绝未知字段、重复键、错类型、浮点/负整数、超范围整数、非法 JSON、尾随额外值。重复键必须在反序列化层检测，不能先转成会丢失重复键的 `serde_json::Value` 再验证。

`command` 闭集为 `inspect`、`reconcile`、`resolve-uncertain`、`promote`、`rollback`。`target` 是严格的 `{ "kind": ..., "id": ..., "namespace": ... }`：`unit` 使用已有 `UnitId` 文本语法；`intent`、`decision` 的 id 为严格 `Sha256Digest`。namespace 复用已有 `namespace_value` 形状：显式 `{ "kind": "Production", "run_id": null }` 或 `{ "kind": "Test", "run_id": "受限文本" }`，缺项/额外项/相矛盾的 run_id 拒绝。这样同一个 Unit 显示 ID 在不同环境不能共用命令身份；namespace 声明依然不授予该环境权限。允许矩阵按 RFC：inspect 三种均可，reconcile 仅 intent/unit，resolve-uncertain 仅 decision，promote/rollback 仅 unit；未知 kind 或错配拒绝。这里只校验形状，不凭 UnitId 文本证明已注册或 namespace 权限。

`expected_version`、`expected_generation` 是必填 u64；纯 wire 不替未注册目标猜版本，也不把 0 自动解释为可初始化或已授权。`requested_at` 必须满足现有 `UtcMicros` 范围，只是捕获请求时间，不是可信当前时间。`reason` 使用现有 `ReasonCode` 闭集；命令执行时再核对其领域语义。

`authenticated_operator_ref` 保留 RFC 字段名及原始受限文本语义，但在类型/注释中明确是 claimed reference，不是认证。Debug/error 不回显该引用；空、超限、NUL、前后空白拒绝。

每个 `evidence_refs` 项为严格 `{ "kind": ..., "version": ..., "protected_uri": ..., "sha256": ... }`。kind/version 为非空、受限、无 NUL 和前后空白的文本；protected_uri 复用现有 `ProtectedRef` 语法；sha256 严格小写 64 hex。类型与版本声明仍需后续闭集来源解释器认证；不在此开放执行能力。保持输入数组顺序作为请求身份的一部分，不排序、去重或静默改写 URI；精确重复项拒绝，避免一条材料被当多个独立证据。空列表可被解析，但是否满足命令最小证据由后续验证器明确拒绝，不能在解析层伪造证据。

### canonical 与稳定身份

新请求 canonical domain 为 `PushOperatorRequest/v1`，复用 canonical-v1；对象含除 `command_id` 外其余九字段，字段名与 wire 一致，嵌套 target/evidence 对象同样按键排序，数组保持原序。`command_id` 必须为此规范字节的 SHA-256；返回完整规范字节及同一 digest，不接受随意非空字符串或自报且不匹配的 hash。重试复用原捕获时间与完整请求，改变目标、版本、代数、dry_run、身份引用、reason、任一证据或时间均得到另一命令身份；这不是防重放持久记录或批准证明。

该新 domain 仅属于未发布的 W18 wire；不改现有 manual trigger 的文本 CommandId、不重算旧 activation journal 或 W15 身份。将来桥接 `ApprovalBinding.command_id` 时可把已校验 digest 原文放入现有文本类型，但批准还必须精确绑定 namespace/Unit/manifest/window/evidence，不能从类型兼容推定已认证。

### 限制、错误与无副作用

输入上限 64 KiB，证据项最多 64 条，普通标识/版本/引用文本采用现有 512 UTF-8 字节上限；超过任一限制返回稳定 typed 错误。允许合法 UTF-8 文本并保留原文，不 trim 或 Unicode 规范化。错误区分过大、wire 结构、字段值、target-command 错配、重复证据与命令身份不匹配；不携带原始 JSON、路径、protected URI 或操作员引用。Debug 同样只显示安全摘要/计数/类型。

本模块不读文件/时钟/环境、不打开连接，不执行请求，不返回 Applied/Inspected/Planned，不生成假的持久审计引用。完整 response 与独立控制面 audit 的绑定由后续真实查询/执行路径产生，禁止为填字段伪造 snapshot hash 或已落盘 audit。

### 验收

1. 先写解析行为反例，缺 module/method 的编译 RED 与真实行为 RED 分开报告；不为制造 RED 先写已知错误生产实现。
2. 独立固定字节与固定 digest 的 golden 请求：至少覆盖 Unit/Intent/Decision、UTF-8/JSON 转义、有序多条证据、true/false dry_run、0 与最大合法数值；预期值不能调用被测 canonical 计算。主线已用不加载项目编码器的 Ruby JSON/Digest 生成三份本工作区取证向量，路径为 `.superpowers/sdd/2026-09-10-operator-request-intake/independent-request-vectors.json`，生成器位于同目录 `independent-request-vectors.rb`。实现者核对后将固定材料/字节/摘要写入正式测试；测试运行时不得依赖 ignored workspace 文件。若交接不含该本地材料，按本计划同一合同用独立标准实现生成并保留过程，不成为构建依赖。向量只证明 wire 期望，不提供认证/权限，也不替代负例测试。
3. 每个参与命令身份的字段发生单项改变但保留旧 command_id 必须拒绝，包括目标 namespace/Test run_id；对象键重排/合法 JSON 空白不改语义身份，数组重排会改变身份；command_id 自身不得进入自身哈希。
4. 拒绝所有字段遗漏/额外字段、外层及嵌套重复键、null/错误类型/负数/浮点/溢出、尾随第二对象、不合法 UTF-8、非法 SHA、错命令目标、重复证据、超限输入及证据数量。合法但未经认证的引用只产生 Unverified 请求，不产生 authority。
5. Debug/error 脱敏测试使用明确的 URI/操作员/路径敏感哨兵，检查不同错误路径均不回显。保留规范字节用于后续批准比较不等于允许日志打印它。
6. 同一个解析 interface 测试正反例，不只测未被入口使用的 helper。序列化/canonical 明确逐字段，避免仅对自身 round-trip 自证。

最终定向命令：`cargo test --lib push_foundation::operator_request_tests -- --test-threads=1`，须报告实际非零测试数；改动文件 rustfmt check；`cargo clippy --lib --message-format=json` 分离既有告警和本批诊断。不跑全仓/真实 monitor/网络，不为只读 parser 测试添加假 transport 调用计数。

实现者交完整报告，主线固定准确 BASE..SOURCE 做独立 Spec/Quality 审查；确认所有输入合同、哈希身份及权限边界后才能将本 Task 标完成。

## 后续依赖与完整目标

请求入口验收后，继续以下必要工作，不把 parser 交付称作控制面已可用：

1. 真实身份/受保护 allowlist、部署/source 根、批准持久化/撤销/重放验证。配置来源需真实平台资料；不能采用未经验证的自由文本、环境变量用户名或 caller 的成功 bool。
2. 应用层在同一已认证 scope 下读取实际 Unit/intent/decision，验证精确请求、最小证据、当前 fence/version/generation；inspect/dry-run/refusal 生成实际 before/投影 after 和独立 audit envelope，业务/durable/activation 库无写入。
3. 受授权独立控制面先持久化审计，再签发 W10 所需精确 audit capability；resolve-uncertain 保持先 inspect、只追加人工处置、原 receipt 不改、绝不重发。
4. promote/rollback 接真实 supervisor、既有事务 CAS/配额与当前 fence；中断/重启按同一 command 精确协调。完整响应 hash、审计 envelope 与 mutation 引用需无循环绑定并经独立 golden 验证，不凭请求 hash 生成成功结果。
5. W15 同一认证快照消费、完整 W17/52 Unit 业务适配、W19 运维安全、W20 故障矩阵、W21 灰度/回滚及真实 CI/生产验收仍必需。

回滚仅撤销本任务新增 module/test 与注册，不修改持久状态或生产进程。Rust 实施后旧 current-source-audit/蓝图仍是固定旧源码视图，须在连贯批次完成后正式刷新，不放宽 freshness。
