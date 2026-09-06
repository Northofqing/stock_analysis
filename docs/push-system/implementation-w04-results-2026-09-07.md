# 推送 Foundation W04 实现与验证结果

**结论：** W04 `RunContext`、`PreparedFacts` 与单次事实捕获合同已经实现并通过切片级验证。active/old 与 shadow/new 可以且必须克隆同一个 `Arc<PreparedFacts>` snapshot；第二次 acquisition 在进入闭包前被拒绝并计数。当前没有接入任何生产 producer/provider/LLM/sink/数据库/scheduler，因此不会改变现有推送行为或 physical owner。

**边界：** 本文只关闭 W04。W05--W21、52 个 Migration Unit、production shadow、physical promotion、durable recovery 和逐交易时段自然样本仍未完成，不能用本文证据替代。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 权威目标

本切片逐项执行以下已冻结依据：

- `docs/push-system/push-system-implementation-rfc.md`：`RunContext` 15 字段、`PreparedFacts` 9 字段、canonical-v1、来源失败不得冒充空结果、重放不得重采集。
- `docs/push-system/push-system-wbs.v1.json` W04：依赖 W01/W03；验收为“old/new 共享同一 PreparedFacts 与捕获模型输出，第二次外部采集有拒绝计数”。
- `docs/Project_Architecture_Blueprint.md` / `.html`：Foundation 零行为变化，shadow 只使用单次采集事实，禁止二次 provider/LLM/业务取数。
- 设计：`docs/superpowers/specs/2026-09-07-push-foundation-w04-capture-design.md`。
- 计划：`docs/superpowers/plans/2026-09-07-push-foundation-w04-capture.md`。

## 2. 提交证据

| 提交 | 内容 | 证据性质 |
| --- | --- | --- |
| `59b8c5b` | W04 单次事实捕获设计 | 先设计，未写运行时代码 |
| `2b671d4` | W04 TDD 实施计划及设计自审修订 | 固定小提交、RED/GREEN、门禁和非目标 |
| `197e67b` | RunContext 合同 RED | 编译器因 `context`/Phase/Trigger/Date/Git SHA 合同缺失失败 |
| `632d330` | RunContext GREEN | 21/21 push_job tests；原 W01 golden SHA 不变 |
| `2921c0f` | PreparedFacts/单次捕获 RED | 42 个缺失合同诊断，覆盖 bytes/time/model/facts/snapshot/state/error |
| `b07b14b` | PreparedFacts/单次捕获 GREEN | 30/30 push_job tests |
| `355c44d` | 双轴 review 修复 | Debug 脱敏、pointer equality、不可复制能力、ObservedAt/AsOf |
| `9404981` | 零接线 fixture 隔离 | 非测试 import 清洁；只对 W06 前明确未接线私有入口定点 allow dead-code |

RED 没有用语法错误、坏 fixture、网络、数据库或既有全库失败充当目标失败；失败符号与随后实现的合同一一对应。

## 3. `RunContext` 15 字段逐行证据

生产结构位于 `src/monitor/push_job/context.rs:301`，全部字段私有，只能只读访问；未实现 `Default` 或通用 `Deserialize`。构造发生在 catalog-bound factory `context.rs:225-273`。

| RFC 字段 | 实现行 | 构造/不变量 | 测试证据 |
| --- | ---: | --- | --- |
| `schema_version` | 302,245 | 固定 `1`，调用方不能传入未知值 | `tests.rs:925` exact field + preimage |
| `run_id` | 303,246 | 受 W01 文本校验；Test namespace 必须与其内嵌 run ID 相等（231-236） | `tests.rs:1016` WrongTestNamespaceRun |
| `unit_id` | 304,247 | 只取 catalog binding，不取 caller payload | golden 字段断言 |
| `namespace` | 305,248 | 只取 binding；Production/Test 闭集 | golden + WrongTestNamespaceRun |
| `business_date` | 306,249 | 取已验证 `OccurrenceIdentityMaterial.business_date`，不取自然日 | golden 精确值与 occurrence SHA |
| `calendar_date` | 307,250 | 独立 `CalendarDate` 严格 `YYYY-MM-DD` | 非规范日期反例 |
| `phase` | 308,251 | `PhaseEpic` 四分支闭集 | Auction 断言；enum 无自由 String |
| `trigger` | 309,252 | Scheduled/Event/Manual 私有分支；与 catalog 注册类型/ID匹配（277-297） | `tests.rs:989` 三分支；`1016` 三类 mismatch |
| `occurrence` | 310,253 | factory 先验证 occurrence family，再由 W01 材料派生 | WrongOccurrenceFamily +固定 SHA |
| `captured_business_time` | 311,254 | 必须显式传入 `UtcMicros`；无系统 now | golden exact integer |
| `activation_generation` | 312,255 | 只取 catalog/activation binding | golden=7 |
| `build_commit` | 313,256 | `GitSha40` 恰好 40 位小写 hex | uppercase/短值反例 |
| `catalog_sha256` | 314,257 | 受校验 SHA，只取 binding | golden 64 位 SHA |
| `source_contract_version` | 315,258 | 只取 binding；facts 再精确复核 | version drift capture 失败 |
| `template_version` | 316,259 | 复用 W02 已有唯一 `TemplateVersion`，不另造同名合同 | golden=`auction-card-v3` |

canonical 字段位于 `context.rs:385`，使用私有 encoder `canonical.rs:10-107`；对象键由 `BTreeMap` 排序，null/string/u64/bool/array/object 有限闭集，没有 serde panic path。固定 preimage 和独立 SHA 在 `tests.rs:925`：

```text
RunContext SHA-256 = ced27f93ce01baa5c775beef415f73fda5b065727ee9d246c91ba9807d7276b8
```

抽取 canonical 后，W01 原 occurrence golden 仍为：

```text
5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a
```

## 4. `PreparedFacts` 9 字段逐行证据

结构位于 `src/monitor/push_job/facts.rs:374-384`；构造器是私有 `try_new`（386-426），外部 adapter 只能先产生受校验的 `CapturedFacts`（256-324），不能直接拼 `PreparedFacts`。

| RFC 字段 | 实现行 | 构造/不变量 | 测试证据 |
| --- | ---: | --- | --- |
| `run_context_sha256` | 375,416 | 从同一个不可 Clone `RunContext` canonical 内容派生 | `tests.rs:1158` 等于 capture context SHA |
| `source_contract_id` | 376,392-396 | 必须等于 capture 私藏的 catalog expected ID | `tests.rs:1220` other-source 失败 |
| `source_contract_version` | 377,397-401 | 必须等于 RunContext 冻结版本 | `tests.rs:1220` v3 drift 失败 |
| `source_refs` | 378,326-359 | 顺序保留；ID 唯一；每项合同 ID 相同 | duplicate + reversed-order success 测试 |
| `canonical_facts` | 379,210-247 | 保存 exact bytes，不 trim/重排/UTF-8 化 | `tests.rs:1078` 空格差异与非 UTF-8 golden SHA |
| `facts_sha256` | 380,414 | 直接复制 exact bytes 已派生 SHA；自身不进入派生输入 | exact SHA + getter equality |
| `provider_observed_at` | 381,111-160,326-359 | 与 source refs 逐项同序；ObservedAt/AsOf 保留；unknown=None | `tests.rs:1103` missing/reversed/unknown 反例 |
| `verified_empty` | 382,402-413,423 | 只从 bound `VerifiedEmptyEvidenceRef` 得到；occurrence/source 必须匹配 | `tests.rs:1252` wrong occurrence 失败、bound 成功 |
| `model_output_refs` | 383,164-208,361-371 | model/version/input/output/protected ref 有序冻结；完全重复拒绝 | `tests.rs:1158` 顺序/重复/共享测试 |

`PreparedFacts/v1` 规范化位于 `facts.rs:469-565`。ExactBytes 在外层只表示 length + SHA；数组保留捕获顺序；SourceTime 把 kind 与 Option value 都编码；facts SHA 不作为自引用顶层字段。相同内容的独立捕获 canonical SHA 相同，但 snapshot 明确不相等。

## 5. W04 核心验收逐点证据

### 5.1 old/new 同一不可变实例

- `PreparedFacts` 不实现 Clone（`facts.rs:373`）；外部不能复制事实对象。
- `PreparedFactsSnapshot` 只封装 `Arc<PreparedFacts>`（569）。
- snapshot clone 只复制 Arc；`shares_instance_with` 与 `PartialEq` 都使用 `Arc::ptr_eq`（571-586）。
- `tests.rs:1316` 证明 active 与 shadow clone 同实例；另一个 capability 即使捕获相同值、canonical SHA 相同，也不是同实例且 snapshot 不相等。
- `ModelOutputRef` 已包含 model/version/input/output/protected ref，snapshot 内只读，shadow 没有模型重算入口。

### 5.2 第二次外部采集拒绝并计数

`PreparationCapture::capture_once` 位于 `facts.rs:649-685`：

1. 先读取状态；非 Open 立即 `rejected_count += 1` 并返回 `AlreadyAttempted`（656-660）。
2. 只有真正进入闭包前才切到 Capturing 并把 attempt 从 0 加到 1（662-664）。
3. typed acquisition failure 与 validation failure 都封为 Failed（665-680）。
4. 成功只创建一次 Arc snapshot 并封为 Sealed（682-684）。

测试证据：

- `tests.rs:1375`：成功后第二个闭包包含 panic，但未执行；calls=1、attempt=1、rejected=1、state=Sealed。
- `tests.rs:1417`：首次 `InputSourceUnavailable` 后第二闭包不执行；typed reason 原样保留、state=Failed。
- `tests.rs:1452`：首次闭包真实 panic 由测试外层捕获；capability 保持 Capturing，第二闭包不执行。
- rustdoc compile-fail：`PreparationCapture` 不满足 Clone，不能复制一个 Open capability 形成第二条调用路径。
- `RunContextFactory::begin_capture(self, ...)` 消耗不可 Clone factory（`context.rs:267`），减少同一 factory 重复发放能力的入口。

### 5.3 来源失败不冒充 NoData

- acquisition failure 只有 `PreparationError::AcquisitionFailed { reason }`，不产生 snapshot。
- `FactsPresence::VerifiedEmpty` 必须携带 `VerifiedEmptyEvidenceRef`；其 occurrence/source ID 在 `PreparedFacts::try_new` 精确复核。
- 失败后的 capture 状态为 Failed，第二次 acquisition 不执行，不能通过“再查一次返回空数组”掩盖首次故障。
- W03 `RetryPolicy` 仍只允许五种发送前输入原因；W04 没有扩张重试 authority。

### 5.4 原始事实与日志安全

- `ExactBytes` 对任意 bytes 直接 SHA，包括非 UTF-8；不做 JSON 重写。
- `ExactBytes::Debug` 只输出 length 与 SHA（`facts.rs:216-223`），不会把持仓、新闻、模型事实正文带进调试日志。
- `PreparationError` 只显示 typed reason/合同错误，不含 canonical bytes、模型输出正文或凭据。

## 6. 双轴 review 结果

### Standards 轴

| 发现 | 风险 | 修复 |
| --- | --- | --- |
| W04 初版重定义 `TemplateVersion` | 两套同语义合同 | 复用 W02 唯一类型，编译期冲突闭环 |
| `ExactBytes` 派生 Debug | 事实正文可能进日志 | 自定义 Debug 只显 length/SHA，并有敏感串反例 |
| snapshot 派生 PartialEq | 两份相同内容可被误认为同实例 | equality 改为 `Arc::ptr_eq` |
| RunContext/PreparedFacts/factory 可 Clone | 可扩散上下文或 acquisition capability | 三者移除 Clone；begin_capture 消耗 factory |
| SourceTime 匿名 Option 时间 | observed_at/as_of 语义丢失 | 增加 `SourceTimeKind`，canonical 纳入 kind/value |
| 非测试 fixture import/wiring dead-code | W04 自身构建噪声 | import 用 `cfg(test)`；只对 W06 前私有入口定点 allow |

### Spec 轴

- RunContext 15/15 字段存在，getter 和 canonical 均覆盖；source-contract ID 未擅自塞入 RFC RunContext，而是封存在 capture binding。
- PreparedFacts 9/9 字段存在；facts SHA 来自 exact bytes；source/time/model 顺序、Option 和 empty evidence 受校验。
- W04 acceptance 的“same immutable instance”和“second acquisition rejection count”都有正向+反向测试。
- Foundation zero behavior 成立：没有 production caller、I/O、clock、provider、LLM、DB、sink 或 scheduler 依赖。
- review 后无未解决的 W04 critical/high/medium finding。

## 7. Fresh 验证结果

| 命令/门禁 | 结果 |
| --- | --- |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS：30 passed / 0 failed / 2855 filtered；panic 行是 catch_unwind 反例，测试为 ok |
| `cargo test --doc push_job` | PASS：2 passed / 0 failed；terminal authority 与 capture non-Clone 两个 compile-fail |
| `cargo check --lib` | PASS；84 条为目标外既有 dead-code warning；无 `push_job` warning |
| `cargo clippy --lib -- -A dead-code -D warnings` | 基线阻断：79 个目标外旧 lint；首个为未修改 `src/data_gateway/futures_delivery.rs:15` |
| `cargo clippy --lib -- -A dead-code` | PASS，exit 0；79 个 warning 均不在 `push_job` |
| 定向 `rustfmt --edition 2021 --check` | PASS：7 个 W01--W04 文件 |
| 5 个 architecture-docs Ruby 验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors |
| `git diff --check c51115e..HEAD` | PASS |
| production-wiring path diff | PASS：`src/bin/monitor`、`src/notification`、`src/durable_delivery`、`config`、Cargo files 均无 diff |

Clippy 基线错误未由 W04 修改，也不在 W04 文件；本文不把它隐瞒为全仓全绿，也不越权批量修理目标外代码。

## 8. 生产 monitor 只读观察

2026-09-07 最后一次只读检查：

```text
PID 20162, etime 04:00:34, state Ss, RSS 206532, command ./target/release/monitor
127.0.0.1:54144 -> 127.0.0.1:18082 ESTABLISHED
10.211.55.2:60076 -> 10.211.55.3:50051 ESTABLISHED
```

它仍是开发前已启动的同一 release 进程；W04 未重启、重建、热替换或接线。TCP ESTABLISHED 只证明连接存在，不证明某一推送被 transport accepted，更不替代 durable receipt/typed authority。

## 9. 零行为变化与剩余工作

相对 W01--W03 结果提交 `c51115e`，W04 源码变更只在：

```text
src/monitor/push_job.rs
src/monitor/push_job/canonical.rs
src/monitor/push_job/context.rs
src/monitor/push_job/facts.rs
src/monitor/push_job/identity.rs
src/monitor/push_job/policy.rs
src/monitor/push_job/tests.rs
```

没有生产 caller，所以当前推送仍完全走旧路径；这既保证开发期不中断现有推送，也意味着 W04 本身尚未修复任何具体 producer 的旧完成/去重/发送问题。

下一步是 W05：在同一 `PreparedFactsSnapshot` 上实现纯 `SemanticProjection`、`JobDecision`、`PreparedPush` 与首次 render exact bytes 封存。W06 才把 machine catalog 注册为 factory 的生产创建者。此后仍需 W07--W21 与逐个 Migration Unit shadow/六门禁/单 owner 晋级，才能把本合同转化为实际推送可靠性提升。
