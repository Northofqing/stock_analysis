# W18 操作员请求入口实施记录

日期：2026-09-10。当前状态：**严格请求入口任务已验收**。修复后16项测试、格式与Clippy验证完成，限定复审两项均关闭且无新重要问题。实施基线为 `8dbd3efe473e13a8fc31d4f4dbfeca71618a4873`；初版源码为 `409dbaff2303fc9c3d6bcbec9d31ee1e9f95f3f8`，最终源码为 `9f35ab18fbba2fda12bba34f5e060a932da39c30`。下文区分初版与修复版证据；29条dead_code Minor仍保留。本文不能作为完整 W18 或生产控制面已经可用的证明。

## 范围与业务意义

本任务落实[请求入口计划](../superpowers/plans/2026-09-10-operator-request-intake.md)：把操作员提交的 JSON 解析、目标约束、证据引用语法和命令身份集中在一个内部入口，输出 `UnverifiedOperatorRequest`。它让后续批准、查询和执行使用同一份已捕获请求材料；解析成功依然不代表操作员身份、来源、Unit 注册或生产权限已认证。

代码仅涉及 [module 注册](../../src/push_foundation/mod.rs)、[请求解析](../../src/push_foundation/operator_request.rs)和[入口测试](../../src/push_foundation/operator_request_tests.rs)。没有 CLI、成功响应、数据库写入、时间读取、外部连接或生产 monitor 改动。

## 输入与命令身份合同

| 材料 | 本任务检查 | 明确不证明什么 |
| --- | --- | --- |
| 十个必填字段及嵌套对象 | 严格结构、类型、重复键、未知键、输入上限 | 不是对任意 JSON 做宽松兼容 |
| command 与 target | inspect 支持 unit/intent/decision；reconcile 支持 unit/intent；resolve-uncertain 仅 decision；promote/rollback 仅 unit | 合法组合不是执行许可 |
| target.namespace | 显式 Production/null 或 Test/run_id，并参与身份计算 | 声明 Production 不授予生产权限 |
| expected_version / expected_generation | 必填 u64，0 是可解析值 | 不替执行时的当前版本、代数和 fence 核对 |
| authenticated_operator_ref | 保留 RFC 字段名，作为受限的 claimed 文本 | 不认证账号或批准者 |
| reason / evidence_refs | 闭集原因；证据精确字段、受限文本、严格 SHA；保留顺序，拒绝精确重复 | URI/hash 不证明受保护存储、可信来源或最低证据充分性 |
| requested_at | 现有非负 i64 微秒范围 | 是请求捕获时间，不是可信当前时间 |
| command_id | 必须等于另外九字段的规范字节 SHA-256 | 不提供持久防重放或批准记录 |

规范域为 `PushOperatorRequest/v1`，紧接 NUL 和既有 canonical-v1 排序 JSON；对象键排序，证据数组不重排。command_id 不进入自身哈希。重试必须复用原请求材料；改变 dry_run、时间、身份引用或任何其他身份材料都需要新命令身份。既有 manual trigger 的文本 CommandId 及历史规范域不变。

上限合同为 64 KiB 输入、64 条证据和 512 UTF-8 字节普通文本；合法 Unicode 保留原文，不 trim 或规范化。错误与新增类型的 Debug 不回显原始请求、操作员引用、URI 或任意文本中的敏感标识；对应最终测试见下节，合同表本身不是运行证据。

## 初版验证与实际迭代过程

独立 Ruby 标准库 JSON/Digest 生成了三个固定字节样本，未调用被测编码器；正式测试嵌入字面量，运行时不依赖临时取证文件。

| 样本 | 规范字节长度 | 固定 SHA-256 |
| --- | --- | --- |
| Unit / inspect / 零值 | 337 | `f43e17c300e257c88c6e96a99ca935091b85e0fc910655e9df98891d760dbd1b` |
| Intent / reconcile / 有序多证据 | 739 | `f3d8af8ac91486b64b9de91c1cd81e5fdae4a2bf697146d1badd24fe2554e28e` |
| Decision / resolve-uncertain / 整数上界与转义 | 633 | `ff89fa2bc88b0a6c223243edaeb5fbb9a76cd0843b3d7ba5626addf82524f8c1` |

首个 Unit 样本实际测试为 1 passed；随后完整模块首批运行命令 `cargo test --lib push_foundation::operator_request_tests -- --test-threads=1` 的结果为 11 passed、1 failed，退出码 101。失败发生在嵌套 OperatorTarget 的 Debug 披露合成 Test run_id 哨兵；这是实际行为反例，不是通过结果。首次缺 module 和中途所有权错误分别属于编译失败，不与运行期行为 RED 混写。

该 OperatorTarget 失败修复后的定向回归已实际取得退出码 0、1 passed、0 failed；它只证明该回归。随后补齐所有合法命令/目标的真实成功入口、结构错类型、证据各字段身份变异、恰好上限及刚超过上限的输入，以及 UnitId 和证据 kind/version 的嵌套 Debug 脱敏。

第二批实际运行同一完整模块命令：13 passed、2 failed，退出码 101。失败为顶层字段顺序数组被当成合法请求，以及 evidence kind/version 的 Debug 泄露合成哨兵。结构反例在顶层数组断言处停止，因此同函数后面的 evidence 数组和 command 对象反例不能称为该轮已实际触发失败。

这两处已修复；随后将同一个内部 [MapOnly 入口](../../src/push_foundation/operator_request.rs#L300)扩展到 target 和全部 namespace 字段，拒绝嵌套对象的数组编码，而不是先转换成会丢失重复键的通用 JSON 对象。实现者最终报告补充了定向 session 19588 的运行期 RED：target 数组被接受，0 passed、1 failed、退出码 101，并记录实际 Ok/Err 断言输出；主线先前检查点只有静态证据，不应当作该轮未执行的结论。后续 Test namespace 数组断言当时未执行，不能同样声称已有 RED。两者在最终完整组中均通过。

初版捕获 `final-3` 执行原完整模块命令：**15 passed、0 failed、0 ignored，退出码 0**。完整测试组覆盖三种目标固定字节、八种合法命令/目标组合、身份材料变异、重复/缺失/额外键、数组/对象错形状、边界与脱敏。初版只有 Production 形状及身份变异反例，独立签名成功样本与单独 Production 数组用例是在下述第1轮修复中补齐，不把 Test 样本充作 Production 正向验证。

初版 `cargo clippy --lib --message-format=json` 捕获 `fix-1`：退出码 0、build-finished=true，完整 808 行 JSON，187 条 warning，其中 28 条来自新模块且均为 dead_code。另 159 条为非目标诊断，未作 BASE 逐条对照，不能全部称为已证明的既有告警。本模块尚无运行时消费者；没有为消除告警添加公开重导出、虚假调用或放宽权限。初审将告警记录为Minor；最终数量及处理边界见下节。

主线只读核对完整输出哈希、命令、工作树、退出码、非零测试计数及全部目标 Clippy 诊断，结果一致；没有重复运行 Cargo。另以独立 Ruby JSON/Digest 重算 13 个新增固定摘要，覆盖合法矩阵、64 条证据、六类 512 字节文本及嵌套 Debug 样本，全部匹配正式测试字面量；这不是另 13 项 Rust 测试。

| 初版验证材料 | stdout SHA-256 | stderr SHA-256 |
| --- | --- | --- |
| final-3 定向测试 | `b111a7e9a8188a879a3a20250663f9e3fa9f42acea580f2f444748c0fcf1db13` | `3661147052dfa425cf7355d6f29f1a0573d6cde925b2977c64e7c14089106590` |
| fix-1 Clippy | `15c8cd28f8ebd301288763240eabdbdc91485eb1b3dd011f8cbd30478812c37f` | `a6bd5f927912589b7672a3810b1239cdf4944160ccbd1c3013d7fbd4e178ac8d` |

实现者报告的 `rustfmt --edition 2021 --check` 对三个改动文件退出码为 0；主线 `git diff --check` 及暂存区检查同样通过。报告记录最终测试/Clippy 前后三文件 SHA 不变，主线按当前文件重算后核对一致；报告中 tests 文件哈希的一处转录错误已更正，代码没有因此修改。

| 初版固定源码路径 | SHA-256 |
| --- | --- |
| `src/push_foundation/mod.rs` | `5e66a5792f7a98f4bd7eed3500bcae7b365e6ea76031daedc2cda255f0110c90` |
| `src/push_foundation/operator_request.rs` | `7d937bbea07327b3a76f33d0fb9ee37863a98df95941294d63f02f563aded283` |
| `src/push_foundation/operator_request_tests.rs` | `cf7ac53c60c09d01f882cebd804add88ff65f6b0c568cdf2f96e0d9c3f72d65f` |

## 独立初审、修复与最终验收

独立 Spec/Quality 审查准确 `8dbd3ef..409dbaf`，未把主线文档或其他实现混入代码包。结论为 Spec 未通过、质量 Needs fixes，Critical 0、Important 2、Minor 1：

1. 初版把未知 command、target.kind、target.namespace.kind 同样归为 WireStructure，不能稳定区分“结构错误”与“结构正确但字段值不支持”。修复应分别使用对应 InvalidField；缺项、额外项、重复键和错类型保持 WireStructure，namespace 的必填 null/string 合同不变。
2. 初版实际已有一条 WireStructure 错误的 Display/Debug 脱敏断言，但没有覆盖多个错误类别。修复补 InvalidField、目标/命令错配、重复证据、身份不符及上限错误的真实入口请求和双格式断言。审查者最初称“完全没有错误格式化测试”，已在定向重读源码后明确更正；不能延续这个错误描述。

修复同时补齐原始 namespace 解码改动后的 Production/null 独立签名正例与数组拒绝。dead_code作为Minor保留到真实消费者接入或最终全分支审查，不通过假调用或扩大公开接口隐藏。主线已核实独立向量、完整命令结果与源码摘要；这些证据不延伸到尚未实现的认证或执行路径。

修复基线为 `409dbaf`，由原实现者独占 Rust/Cargo完成。只修改两个已有请求解析/测试文件，module注册、历史身份、权限和所有生产路径未改。修复后证据如下，已通过仅针对修复差异的独立复审。

第1轮修复前的实际入口回归：同一模块命令执行16项，14 passed、2 failed、退出码101。明确失败为未知command与未知target.namespace.kind仍返回WireStructure，而预期对应InvalidField；同一函数后面的target.kind断言当时尚未执行，不能称为第三项已复现失败。Production/null独立正例、Production数组拒绝以及七类错误脱敏在此轮已通过，说明这些新增测试是有效覆盖，不能描述为它们各自发现了产品故障。

修复后捕获 `fix-1` 执行相同模块命令：**16 passed、0 failed、0 ignored，退出码0**，耗时674.141秒。未知command、target.kind和target.namespace.kind均返回对应InvalidField；原始wire结构仍通过MapOnly和deny_unknown_fields检查。run_id使用必填的null|string原始类型，不依赖Option缺省。新增Production/null独立样本322字节，SHA为 `4f80bb38823491535dcf9df0646c647df47aa1f95b1f2c4df7b83d077f5bbc99`，仅证明未经授权请求的解析。

修复后 `fix-2` Clippy退出0、build-finished=true，完整809行JSON、188条warning；其中29条目标诊断均为dead_code，比初版增加的1条来自私有WireRunId。未隐藏告警、未扩大公开接口，非目标159条仍未作BASE逐条对照。定向rustfmt/check退出0，主线diff/cached检查通过；报告记录测试/Clippy前后源码SHA一致，主线重算核对一致。格式命令准备中两次执行错误未触及文件或启动Rust检查，随后成功命令和源码摘要已记录；它们不属于行为RED。

| 修复版材料 | SHA-256 |
| --- | --- |
| parser源码 | `0f0a29a16c62c9eab537d110fe7a04cdba6cef1682441ef6df6cbb85697250dd` |
| tests源码 | `6302e702eb8b34719e77d1472cb90cf90ea74cc8e72c336d5d420bf787a66b63` |
| fix-1测试stdout | `6794a2deffc4dd1eeb0f535883c3c4c710db1d0c4a50ffa940ce2348113ed339` |
| fix-1测试stderr | `44a7579f6268966060fc4ce0f51930b7d40a09412199547ea4ede4b0091210f1` |
| fix-2 Clippy stdout | `da19ed24e05c67c83d9c934caf3009dd83172cecd1063192648f5b00d0a2a73b` |
| fix-2 Clippy stderr | `13aceee67b841b7fe53be99e7777d7f34343dd4b02a770604cdc7d84670f7a2a` |

主线只读核对完整输出与summary的命令、工作树、退出码、计数、SHA及全部目标诊断，结果一致；未重复运行Cargo。限定复审范围为 `409dbaf..9f35ab1`，结论：**两项ADDRESSED，无新Critical/Important、无范围外观察**。复审核对了错误分类、七类脱敏、必填namespace、对象/重复键、Production正例与数组回归以及同源码身份。

因此本请求入口Task完成；保留最终29条dead_code Minor，在后续真实消费者或最终全分支审查处理，不把它描述成干净无告警的lint。没有部署或替换生产二进制，没有实际请求执行或人工处置数据写入。

## 完整目标仍需完成的接线

请求入口之后仍须接真实身份与受保护 allowlist、批准/撤销/持久防重放、同一认证 scope 下的实际查询与版本核对、独立持久审计及 W10 精确 audit capability、resolve-uncertain 的只追加人工处置，以及 promote/rollback 的 supervisor/CAS/共同 fence。请求哈希不能替代实际响应快照或持久审计引用。

W15 真实认证与共同快照消费、[P-02 真实业务影子接线](p02-shadow-integration-handoff-2026-09-10.md)、其他 Unit 迁移、W19 运维安全、W20 故障矩阵和 W21 真实灰度/回滚验收仍在总目标内。此任务完成后也不能把这些工作归为“只剩测试和文档”。本轮不访问真实环境，不启动、停止或监控生产 monitor。
