# W19 原始接受时钟与持久化最终化延迟联查

日期：2026-09-08。状态：实施中。此计划交付 W19 的持久化 SLA 检查，不关闭保留期、安全审计、生产探针/晋级接线或完整 W19。

## 目标与规范

按 `docs/push-system/push-system-implementation-rfc.md:504,1253,1417` / WBS `W19-contract`，从真实持久化 receipt 的 Accepted 时间到业务 Completed 转换时间报告延迟；未最终化时从原始 Accepted 到显式观察时钟计算。目标两个 reconcile 周期，五分钟硬上限。不能用创建、重查、资格转换、手工接受或日志时间代替原始 transport Accepted。

现状：Generic/P01 的 `sink_results.accepted_at` 已经和 exact canonical TypedReceipt 校验；N02 terminal audit 保存 `news_flash_remote_receipt.accepted_at`。现有 Foundation/W09 输出丢失这个已验证时间。业务 `push_intent_transitions.occurred_at` 保存 Completed 时刻及 terminal ref/binding/disposition，但两次独立 inspect 不能伪称同一业务快照。已有 `query_intent` / `query_transition_chain` 必须复用，不能再写一套弱 SQL 校验。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 的 `codex/push-reliability-20260905` 开发；不操作根工作树、生产 monitor、真实数据库、provider、LLM、sink、PAM、订单、owner、`.env` 或生产部署。
- 不修改冻结 25 对象 SQL、八份 RFC 输入、目录/WBS/蓝图、既有 canonical v1 字节/域/哈希或持久化协议；不新增依赖、自动修复、发送、清理、晋级或默认生产身份。
- 复用现有实际 authority 校验和业务转换链校验；报告仅为指定来源/显式观察时钟下的结构化事实，不是生产路径认证、跨库原子快照、可信时钟或晋级授权。
- 父代理独占 Cargo/Git、push_foundation/mod.rs 注册及中文文档；实现代理仅改授权源码/测试/report，不跑 Cargo/Git、不起其他代理、不访问网络/生产。apply_patch 编辑，定向 rustfmt，禁止全仓 cargo fmt。

## Task 1: 原始时钟贯通、只读业务联查与 SLA 结果

### 完成条件

实际通过现有 Generic、P01、N02 持久化 authority reader 和业务 intent/完整转换链读取器，得到绑定同一 intent 的结构化 SLA 检查结果。至少一组每类 authority 的测试由真实临时存储写入/重启/重查，而非只拼自报 timestamp；实际检查不调用发送/恢复/业务写入。不能只交独立纯时间计算函数或没有消费者的时钟字段。

### 文件所有权与复用位置

- 新增 `src/push_foundation/finalization_sla.rs`、`finalization_sla_tests.rs`。父代理注册并按需要仅导出内部接口，生产 operator CLI 尚无授权接线。
- 可修改 `src/push_foundation/intent_store.rs`，以一个只读事务读取 actual snapshot + 全链，复用私有 `query_intent` / `query_transition_chain`。可新增此文件的私有 child module承接这些读取逻辑，路径先通知父代理；不另建 SQL/verifier/opener 栈。优先在 source-owned 既有连接上开启 DEFERRED 只读查询事务，不用写入型 open 冒充纯只读 opener，不改 PRAGMA/schema。已有事务冲突要返回错误，不能偷提交调用方事务。
- 可修改 `src/push_foundation/generic_transport.rs`、`dedicated_transport.rs`，将现有实际 source reader 的单次结果与原始接受时间供 SLA 检查消费。复用原 exact map/binding 校验，不复制或弱化它们；现有 verify_p01/verify_n02/Generic TerminalAuthorityPort 合同保持不变。
- 为暴露已经验证的时刻，可必要地修改 `src/durable_delivery/model.rs`、`coordinator.rs`、`src/event/mod.rs` / `envelope.rs` 中现有 read model、其构造与读取器。只加非持久化读取元数据/私有 getter或解码复用，不改磁盘格式、哈希域、实际投递/时间采集策略。模型字面量受影响时，可机械更新对应现有测试；禁止广泛重构。
- 定向参考真实来源：coordinator.rs:2929,2978,6300–6497,7107–7309；dedicated_transport.rs:212–569；event/mod.rs:615–705,975–1185；intent_store.rs:1964–1979,2328–2481；现有 generic_transport_tests / dedicated_transport_tests / terminal_authority_tests / business_finalizer_tests 的临时 fixture。

### 行为要求

1. 查询输入显式绑定 intent_id、现有 policy/template/authority route、观察时钟和非零 reconcile 周期。N02 必须有显式路由/window，不能从查询时间或显示文本猜 window；验证它与真实 envelope/intent 材料的既有 exact 连接。该路由参数不是已认证注册的生产 route 声明。未知/不匹配路径不能退化为 Generic 或未发送成功。
2. 在一个业务读取事务中校验实际 intent 和完整链，拿到版本/head及最早合法 Completed 事件。Completed 的 terminal_ref、terminal_binding_sha、disposition 必须与当前同一 authority 终态精确一致。保留当前 ResolutionRequired 等未决状态和既有 Completed 历史，不能因历史有完成事件就称当前已就绪。不同 terminal 历史或材料漂移要返回结构化冲突，不跳过不匹配事件只挑有利记录。
3. 通过已有真实 authority 校验取得 Accepted 的原始时刻：Generic/P01 为 exact TypedReceipt；N02 为 exact terminal envelope remote receipt。最终输出的时间必须与用于 W09 exact terminal binding 验证的同一份实际来源材料关联。不得读取宽松 JSON 拿 timestamp 后跳过原 schema/hash/attempt/disposition/channel/append-seal 校验。需要前后两次查回时比较同一 evidence/binding/ref，漂移拒绝，不宣称跨库原子。
4. 原始 Accepted 即使尚未推进到 AwaitingFinalizer（例如仍 AwaitingAuthority、确认丢失），也要报告待最终化并计算年龄；不得忽略这些跨库崩溃窗口。合法 non-Ready 初始 intent 可明确 NotApplicable；Ready 来源后来转 NoData/Disabled 但 authority Accepted 等矛盾不得当 NotApplicable。Completed/manual、Rejected、Uncertain、ManualNotDelivered、Missing、PendingSeal、ResolutionRequired、来源错误/损坏各有明确状态，不成为 TransportAccepted 成功样本。PendingSeal 不从未封存行提取一个看似可信 Accepted 时间。
5. 时间计算使用 checked UTC 微秒/Duration：reconcile 周期必须非零、倍增/换算不溢出；不取当前系统时间作隐藏默认。缺少或无法转换时刻、观察时间早于 accepted/链上最新事实、Completed 早于 Accepted 均报告 ClockUncertain/明确拒绝，不产生负数或 clamp 到0。该检查只验证相对已存事实的时序，不声称已经部署跨重启可信时钟/回拨水位；该外部保障仍属 W16。
6. 已 Completed 的延迟固定取原始 Accepted 到最早精确匹配 Completed，不随之后查询/同态事件增长或重置；未完成取 Accepted 到观察时钟。报告保留 elapsed、two-cycle target、目标是否超过（elapsed>2*cycle）和硬上限是否到达（elapsed>=300s），分别表达边界事实，不授予晋级。未决到达硬上限必须标记需阻断；已完成样本仍保留其到达/超出阈值事实，不能把迟到历史改成按时。微秒精度规则和等号边界需写明并测试。
7. 报告私有构造、字段只读，包含 namespace/Unit/intent/decision、当前业务状态、version/head、authority类别/状态、相关不可变证据引用或hash、原始接受/完成/观察时间和阈值事实；不能把调用方预置的成功布尔作为权威。Debug/Error不得泄漏正文、持仓、模型内容、webhook/key、任意数据库错误、路径或远端回执 message_id。稳定 reason 使用现有 FinalizerDeadlineExceeded / TerminalRefInvalid 等适用闭集；不要修改全局 ReasonCode 注册。
8. 查询前后不改变业务/durable/audit行、不获取恢复lease、不调用sink/append/finalizer。允许已存在 source-owned 连接管理自己的只读事务；不为观察新建生产库、不checkpoint/复制WAL主文件。完整保留期与清理资格稍后单独实现，本任务不删除任何证据。

### 测试与检查

使用隔离 Test namespace 和唯一临时数据库/审计目录，不读取真实数据。将实际已有 writer/terminal reader/finalizer 作为 fixture 建立已提交记录，断言观察前后全表/append计数不变并保留原sink调用次数。覆盖：

- Generic/P01/N02 各至少一个真实原始接受时间；重启后仍取原时刻；观察时间、created/updated/资格转换时间不同也不能替代；N02错窗口/缺路由拒绝。
- 未最终化（包含 authority Accepted、business仍AwaitingAuthority/PendingDispatch）、合法最终化、人工接受、Rejected/Uncertain/NotDelivered、Missing/PendingSeal；Ready非发送状态矛盾与当前ResolutionRequired不得被成功历史隐藏。
- Completed必须精确ref/binding/disposition；损坏链、错误namespace/Unit/intent/来源、终态漂移/错误channel/schema/hash均 fail closed；复用既有验证覆盖无需复制全部旧测试。
- 周期0/换算溢出，two-cycle前/等于/后一微秒，300s前/等于/后一微秒；观察早于来源/链最新事实、Completed早于Accepted；完成后再次查询延迟不增长。
- 查询实际不写入、不发送、不产生append、不推进游标；Debug/Error脱敏。

父代理待源码冻结后运行目标/相关消费者合批（根据实际修改测试模块名调整，必须实际运行数量>0）：

```sh
env CARGO_PROFILE_TEST_INCREMENTAL=true cargo test --lib -- --test-threads=1 push_foundation::finalization_sla_tests:: push_foundation::generic_transport_tests:: push_foundation::dedicated_transport_tests:: push_foundation::terminal_authority_tests:: push_foundation::business_finalizer_tests::
cargo clippy --lib --no-deps --message-format=json
```

若修改实际 durable/event reader，合批还须纳入该 reader 既有覆盖测试。父代理定向fmt/diffcheck/RFC输入校验；163条既有libClippy/43条testwarning单列，改动文件不新增告警。原实现代理修复失败，最终证据不得沿用已被修复改动影响的旧结果。

### 实现报告

写同目录 task-1-report.md：实现/实际数据流、公开或内部导出名、测试、文件列表、自审、未解决项；父代理独占验证与Git，未运行测试必须明确。若无法为某类 authority取得实际绑定时刻，指出具体缺口，不以统一None/自造时间完成任务。冻结后父代理测试/提交，再做固定原始BASE..HEAD独立Spec+Quality审查。

## 保留的后续范围

W19 仍需指标汇总/生产健康和晋级消费、Uncertain 人工 SLA、保留期/法律保留/备份完整性/WORM边界和安全审计。W15/W16真实身份与来源、完整W17端口接线、W18/W20/W21、52Unit迁移和实际发布门禁不变。
