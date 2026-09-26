# Task 4 实施报告：NewsAI v3 身份与 legacy 兼容

基线：`a8c791cee92047a8c02978c1511611cf68d28616`。开始时 tracked 工作树干净。

## 约束与设计裁定

- 唯一规格：`task-4-brief.md`；预检只作参考。仅 Task4 文件；不部署、不发送、不访问生产数据库。
- 使用 executing-plans / TDD / karpathy-guidelines / codebase-design。围绕同一个业务身份接口逐个行为 RED → GREEN。
- 经主审确认：不扩 schema/catalog。已有 immutable `news_ai_delivery_recovery_snapshot.fact_snapshot` 使用严格带 tag/version 的 v3 envelope；legacy snapshot 字节不动。immutable assessment 的 analysis format 精确区分格式，v3 必须有 envelope，未知/缺失/损坏拒绝。`source_identity_sha256` 直接绑定 canonical identity bytes，再进入原 row/chain hash。
- assessment、envelope、chain、frozen card/fact 同一个 SQLite immediate transaction。新格式只来自 live 分支；不重写历史身份。

## TDD 日志

1. RED：`cargo test --lib monitor::news_ai::tests::v3_business_identity_is_independent_of_batch_evidence -- --nocapture`。旧 request 身份验证同正文仅 batch/display 改变：真实断言失败，1 failed，digest `a4c6a3…` 与 `700480…` 不同，exit 101。首次编译 3m47s。随后增加显式 v3 profile/identity/request 接口，legacy `try_new` 保持原合同；同一反例改用新增接口验证。
2. Legacy golden：切换 writer 前，从旧实现的隔离 SQLite 生成 v1/v2 assessment/chain/delivery/card/snapshot 完整逻辑 SQL 与 prompt 字节，后续固定为 fixture，不由新版 writer 重造。
   - 首次 `cargo test --lib database::news_ai::tests::capture_legacy_v1_v2_fixture_before_v3_writer -- --nocapture` 是 fixture helper 错误，非功能 RED：表名 `news_ai_delivery_chain` 应为 `news_ai_delivery_event_chain`，导致 SQL `near ||`。已修正。
3. `cargo test --lib database::news_ai::tests -- --nocapture`：31 passed / 1 failed。旧 writer golden 捕获成功；真实 RED 为 v3 append 被旧 ID 校验拒绝，`InvalidInput("assessment_id differs ... expected 754d1f...")`。未开始数据库 v3 writer 修改前，已把 SQL 和两个完整 prompt 固定到 `src/database/fixtures/news_ai_legacy_v1_v2.json`。之后移除生成器，回归只加载固定 fixture。
4. `cargo test --bin monitor news_ai_shadow::tests::v3_same_tick_keeps_distinct_text_revisions -- --nocapture`：真实 RED，1 failed，`left: 1, right: 2`。旧 batch/item key 合并了同 tick 两个 content revision；编译 3m43s。
5. DB codec/同事务/混合恢复实现后：`cargo test --lib database::news_ai::tests -- --nocapture`，32 passed，0 failed。包括真 SQLite reopen、旧所有 SQL bytes/prompt 不变、legacy manual/ready 与 v3 ready 共用公平队列。编译 3m42s，测试0.37s。
6. 增补调用计数与故障边界后，同一 DB 限定命令首次因测试 closure `async` 借用 identity 出现 E0373，编译失败，未运行测试。修为 `std::future::ready` 同步私有 SQLite 边界后重新验证；这是测试编译修正，不冒充业务 RED。
7. 最终 bin 限定测试第一次为13 passed / 1 failed，唯一失败是旧测试硬查源码 `candidate_execution(existing.is_some())` 与 `.assess(&request)`，接口接线后字符串不再存在；不是模型被错误调用。删除该脆弱断言，改为真实 `assess_candidate` 调用、disabled capability、计数 receipt provider，并将 live capability 的拒绝前置到任何 DB/市场/模型尝试之前。Task1 无模型恢复仍由独立scanner负责；随后相同命令14/14 GREEN。

## 实现与调用迁移

- `NewsAiAnalysisProfile` 在模型调用前确定配置 provider/model、analysis、prompt/system digest、profile revision、data contract；实际响应 model 仍取 receipt。`NewsAIAnalyzer::assess` 校验 request profile 与实际选中的配置相符，不冒充上游返回模型。
- `NewsAiIdentityV3` 唯一计算文本修订/业务 identity。title/summary/content 带 Option 标记及长度前缀原 UTF-8 字节；不做语义/HTML/Unicode/大小写归一化。source batch、observed_at、market/evidence、显示名称不进 identity。
- `NewsAIAnalyzer::assess_if_absent` 是 pre-call barrier：同一 typed identity 经过持久 lookup、lazy market/request preparation、模型调用。命中或 lookup error 都不会做市场采集或模型调用。bin exact-candidate、terminal lookup、pre-call lookup、request 使用该身份；命中原 assessment 交回 Task1 recovery owner。
- 持久 row 的 `analysis_version` 使用严格 `news_ai_identity_v3/<analysis version>` 格式标记；真正 analysis version 在 identity profile，恢复/卡片仍呈现原分析版本。整个保留 family 的未知格式拒绝。
- 现有 recovery snapshot 表仍 schema_version=1；legacy `fact_snapshot` 原 bytes 不变。v3 以 `identity_version=3` 和固定 codec tag 的 envelope 同时保存 canonical identity 与原 fact snapshot 字符串，严格重编码验证并拒绝 unknown fields。
- `source_identity_sha256` = 带域隔离的 v3 canonical identity hash。v3 独立 content hash 同时绑定完整 canonical assessment 与 envelope 原 bytes，防止只改 batch/observation 再重算 snapshot SHA。原 row serialization/chain/domain 与 legacy content/source hashing 保持不变。
- assessment/envelope/chain/frozen card 在一个 immediate transaction。原始 raw append 拒绝 v3 半条材料；仅完整 audited append 可写 v3。重复不同 receipt/evidence 仍 Conflict，不将第二次调用材料与第一次链混装。
- v3 缺 snapshot/card、未知/损坏/移挂材料在 chain validation 就拒绝，claim 不推进；legacy 缺材料仍保持原 manual reason，不制造新的提示或回填。
- 不改 schema/catalog、notify/counting，也不另建扫描器或队列；Task1 的 claim 排序、全局 limit、manual ack、shared permit 保持原实现。

## 最终限定验证

- `cargo test --lib database::news_ai::tests -- --nocapture`：35 passed / 0 failed，3m44s 编译、0.38s 执行；包含实际关闭重开、第一批 frozen materials 不变、第二次模型/市场调用0、损坏不推进claim、四阶段事务失败回滚。
- `cargo test --lib monitor::news_ai::tests -- --nocapture`：39 passed / 0 failed，1.70s复用编译；新增身份、原UTF8修订/缺失标记、各合同版本、严格codec、configured≠actual模型测试均通过。
- `cargo test --bin monitor news_ai_shadow::tests -- --nocapture`：14 passed / 0 failed，35.01s，执行0.01s。新增同tick正文修订保留、跨batch同正文候选合并、disabled模型0调用；Task1无live ingress、无模型、无发送能力与shared worker恢复测试保持通过。
- 总计88项限定测试通过；没有跑无关全量、release、额外check/build/clippy。已有未使用代码warning未扩范围修正。
- `git diff --check`：最终diff通过（包括报告）。

## 改动文件与关键证据

- `src/monitor/news_ai.rs:814`：profile；`:867`：唯一v3 identity/codec；`:1043`：v3 request；`:2201`：调用前查重与lazy request/model barrier。
- `src/database/news_ai.rs:793`：分格式canonical校验；`:872`：v3完整envelope内容绑定；`:896`：精确持久格式分派；`:1005`：必需材料读取；`:2223`：同事务完整append；`:2347`：只从首次冻结材料恢复的v3 lookup。
- `src/bin/monitor/news_ai_shadow.rs:561`：exact candidates采用同identity；`:682`：live owner接线，disabled提前拒绝；原Task1 scanner/claim公平调度不变。
- `src/database/fixtures/news_ai_legacy_v1_v2.json`：从旧writer捕获的固定人工测试金样，含assessment/chain/delivery/card/snapshot及完整prompt，没有生产数据。
- 本报告：实施证据与后续边界。progress仅追加状态，不随本次提交暂存。

## 自审与未覆盖

- 没有改旧 DDL、CatalogV2、PersistedAssessmentRow 的 Serialize 字段、legacy hash域、旧system prompt常量。固定legacy fixture不是使用新writer重新生成。
- 没有调用模型/推送/行情网络；计数模型为隔离测试 Adapter。没有读取或写生产库、没有启动/重启monitor、没有部署或迁移。
- 当前是源码格式升级：含v3记录的库会被旧binary拒绝。Task10必须处理升级/回滚边界；不能以schema未变就宣称旧binary可读取v3，也不自动把历史v1/v2改为v3。
- 整链校验与pending扫描仍是O(n)，v3多一层恢复材料校验；没有大历史规模性能证据。保留现有fail-closed安全语义，后续优化不能跳过审计链。
- configured profile若被上游静默路由至另一实际模型，同profile仍去重；actual model原样保留receipt，显式配置模型/profile revision变化才形成新identity。这是已批准的业务口径，不做虚假上游模型承诺。
