# 推送 Foundation W04 单次事实捕获设计

**状态：** 已设计，待按测试先行实现；不接生产 caller、provider、LLM、数据库、sink 或 scheduler。

**决策日期：** 2026-09-07

**范围：** W04 `RunContext`、`PreparedFacts`、单次外部采集和旧/新投影共享事实。本设计建立 W05 投影、W06 catalog、W17 shadow 的输入边界，不宣称 W04 已实现，更不宣称 W01--W21 或 52 个 Migration Unit 已完成。

## 1. 结果

在既有 `stock_analysis::monitor::push_job` 深模块内新增两类不可变合同：

1. `RunContext` 一次性冻结业务日、自然日、phase、trigger、occurrence、业务时钟、activation generation、build、catalog、来源合同和模板版本。
2. `PreparationCapture` 对一个 `RunContext` 最多执行一次事实 acquisition；状态一旦离开 `Open`，任何再次调用都在进入外部闭包前拒绝，并增加可观察拒绝计数。
3. 首次成功返回 `PreparedFactsSnapshot`；其内部是 `Arc<PreparedFacts>`，active/old 与 shadow/new 克隆的是同一个不可变实例，不是内容相同的两份重建对象。
4. 来源引用、来源时间、规范事实原始字节和模型输出在首次 acquisition 中一起捕获。shadow 不得重新访问 provider、业务查询或 LLM。
5. 来源失败保持 typed error；不能通过空数组或 `verified_empty=true` 把失败伪装成 NoData。
6. 本工作包没有 production wiring，因此不会改变当前 release monitor、物理发送 owner、消息内容、游标或数据库。

WBS 验收“old/new 共享同一 PreparedFacts 与捕获模型输出，第二次外部采集有拒绝计数”被直接编码为类型和测试，而不是依赖日志约定。

## 2. 权威依据与证据映射

| 设计点 | 权威依据 | W04 可执行证据 |
| --- | --- | --- |
| `RunContext` 15 个字段逐项冻结 | RFC“类型：RunContext” | 构造测试逐字段读取；schema 固定 1；跨字段/格式错误拒绝 |
| `PreparedFacts` 9 个字段逐项冻结 | RFC“类型：PreparedFacts” | exact canonical bytes、派生 SHA、来源版本绑定、空结果证明、模型输出测试 |
| old/new 使用同一实例 | 蓝图 §24 Foundation、RFC 适配器一致性合同 | 两个 snapshot 的 `Arc::ptr_eq` 为 true |
| 禁止 provider/LLM 二次调用 | RFC“影子副作用”、W04 acceptance | 计数闭包只执行一次；第二次捕获返回拒绝且闭包计数不变 |
| 失败不能当空结果 | RFC `empty_source=VerifiedEmptyOnly`、Q85 | `VerifiedEmpty` 必须携带 coverage evidence；acquisition error 不产出 snapshot |
| 重放不能重捕获 | RFC 公共类型规则、Q33/Q72/Q78 | capture 状态在成功和失败后均封闭，不读取系统 now |
| Foundation 零行为变化 | 蓝图“Foundation Release（零行为变化）” | 模块只依赖纯值/`Arc`/SHA；没有 provider、DB、sink、env、clock 接线 |

权威文件：

- `docs/push-system/push-system-implementation-rfc.md`：流程、RunContext/PreparedFacts 字段、适配器一致性、影子副作用。
- `docs/push-system/push-system-wbs.v1.json`：W04 依赖、估算和验收。
- `docs/Project_Architecture_Blueprint.md` / `.html`：深模块位置、Foundation 零行为和 shadow 共享事实边界。

## 3. 方案比较

### 3.1 采用：一次性 capture capability + 共享不可变 snapshot

`PreparationCapture` 同时保存 `RunContext` 和内部状态，首次调用时把状态由 `Open` 原子地移到 `Capturing`，再进入调用方提供的 acquisition 闭包。成功后成为 `Sealed(snapshot)`；失败后成为 `Failed(error)`。两个终态都禁止再次采集。

优点：

- “只能一次”由能力对象执行，不靠调用方纪律；
- 第二次调用在外部闭包之前被拒绝，可精确证明 provider/LLM 没有二次调用；
- old/new 克隆同一个 `Arc`，模型输出不会被重算；
- W17 可在这一边界外增加八类副作用拒绝 capability，不需要重做事实模型。

### 3.2 不采用：`prepare()` 每次自行调用 provider

即使两个结果的 SHA 相同，也无法证明 provider、LLM 或业务查询只调用了一次；两次读取还可能跨 batch、跨时间或得到不同股票集合。这正是集合竞价、新闻 same-tick 和模型结果漂移的现有风险来源。

### 3.3 不采用：只缓存 JSON 字节

只缓存 JSON 无法冻结 `SourceRef`、未知来源时间、模型输入/输出引用和 `verified_empty` coverage，也不能证明 active/shadow 使用同一对象。它会把重要 lineage 留在日志或调用栈中。

### 3.4 不采用：在 W04 直接接现有 provider

W06 尚未提供 65 kind/102 producer/52 Unit 的闭合 catalog，无法证明调用方 producer、Unit、completion owner 和 source contract 绑定正确。W04 只提供安全 seam；production adapter 必须等 W06 注册和逐 Unit shadow 后接入。

## 4. 模块与依赖

```text
src/monitor/push_job.rs                  唯一公共 seam / re-export / error
└── src/monitor/push_job/
    ├── canonical.rs                    canonical-v1 私有编码和 domain hash
    ├── identity.rs                     W01 身份，复用 canonical.rs
    ├── context.rs                      W04 RunContext 与 trigger
    ├── facts.rs                        W04 fact input、snapshot、单次捕获
    ├── delivery.rs                     W02（不改变）
    ├── policy.rs                       W03（只复用 verified-empty evidence）
    └── tests.rs                        仅通过公共 seam 验证 W01--W04
```

允许依赖：现有受校验身份值、`std::sync::Arc`、标准集合、`sha2`。

禁止依赖/行为：系统当前时间、环境变量、文件系统、数据库、provider SDK、LLM、HTTP/gRPC、notification、durable 写入、cursor、订单、线程任务或全局 singleton。

## 5. canonical-v1 复用

W01 的私有 canonical JSON 编码迁移到 `canonical.rs`，W01 golden vector 必须保持不变。W04 只允许有限、显式的 canonical 值：`null`、字符串、非负整数、数组、对象。规则保持 RFC 原文：

- preimage 为 `<ASCII domain><0x00><canonical UTF-8 JSON>`；
- 对象键字典序、数组保持捕获顺序；
- Option 必须编码为 `null`；
- 整数最短十进制；不支持 float；
- string 按 JSON 规则转义；无无关空格和末尾换行；
- domain 分离，`RunContext/v1` 与其他对象不能发生跨类型碰撞。

该编码器仍保持模块私有，不能成为“任意 JSON 转权威 hash”的公共工具。

## 6. `RunContext`

### 6.1 值类型

新增受校验类型：

| 类型 | 约束 |
| --- | --- |
| `CalendarDate` | 严格规范 `YYYY-MM-DD`；与 `BusinessDate` 语义不同 |
| `PhaseEpic` | 仅 `Preopen/Auction/Intraday/Postclose` |
| `Trigger` | 仅 Scheduled/Event/Manual 三种受校验分支 |
| `ScheduleId`、`CommandId`、`AuthenticatedOperatorRef` | 复用 W01 文本规则且语义不可互换 |
| `GitSha40` | 恰好 40 位小写十六进制 |
| `TemplateVersion` | 复用受校验非空文本规则 |

`Event` 触发包含 `producer_id` 与一个完整 `SourceRef`，不能只保存展示名称。`Manual` 触发必须同时包含 `command_id` 和已认证 operator ref；W04 不负责认证，仅接受上游认证边界生成的受校验引用。

### 6.2 catalog binding 与构造入口

`RunContext` 的字段私有且不可修改。公共读取接口不暴露可变引用，不实现 `Default` 或通用 `Deserialize`。

构造由 `RunContextFactory` 完成。它的直接装配入口保持 crate-private，并要求一个不可变 `CatalogRunBinding`，至少绑定：

```text
unit_id
allowed producer/trigger
namespace rule
occurrence family
activation_generation
build_commit
catalog_sha256
source_contract_id + source_contract_version
template_version
```

调用方只能提供本次捕获值：`run_id`、business/calendar date、phase、trigger、`OccurrenceIdentityMaterial`、captured business time。factory 先验证 occurrence family 等于 binding 注册家族，再派生 RFC 所需的 `OccurrenceId`；已经散列的不透明 ID 不能反向证明它来自正确家族。Test namespace 内嵌的 run ID 还必须等于本次 `run_id`。factory 完成这些校验后构造 `RunContext`。

`RunContextFactory::begin_capture` 返回 `PreparationCapture`，后者除公开可读的 `RunContext` 外，还私有保存 catalog 给出的预期 `source_contract_id`；这是必要的，因为 RFC 的 `RunContext` 只有 source-contract version、没有 ID。首次 captured facts 的 ID 和版本必须分别与这两个冻结值一致。

W04 测试使用 crate-private catalog binding fixture；W06 实现正式 catalog lookup 并成为 factory 的唯一生产创建者。不得为了方便把 RFC 未声明的 `source_contract_id` 塞进 `RunContext` canonical 对象。

### 6.3 规范化与摘要

`RunContext` 精确编码 RFC 的 15 个字段，`schema_version` 固定为整数 1。`Trigger` 使用带显式 kind 和完整字段的对象编码，未出现的分支字段不混入另一个分支。`sha256()` 每次对不可变内容得到相同结果；没有缓存时钟或运行时 metadata。

## 7. 来源和模型引用

### 7.1 `SourceRef`

字段：

```text
source_ref_id
provider
external_id
source_contract_id
content_sha256
```

所有文本受校验。`source_ref_id` 是捕获集合内的稳定引用；`source_contract_id` 必须与本次 `PreparedFacts` 合同相符。`Vec<SourceRef>` 保留适配器已冻结的顺序，同时拒绝重复 `source_ref_id` 和完全重复引用；构造器不擅自排序，因为顺序属于捕获事实。

### 7.2 `SourceTime`

字段：`source_ref_id` 和 `observed_at: Option<UtcMicros>`。列表必须与 `source_refs` 一一对应且顺序相同；未知时间编码为 `null`，绝不补当前时间。

### 7.3 `ModelOutputRef`

字段：

```text
model
version
input_sha256
output_sha256
protected_ref
```

列表保留捕获顺序并拒绝完全重复项。它只保存可审计引用和摘要，不保存凭据，也不允许 shadow 重新调用模型。

### 7.4 exact facts bytes

`ExactBytes` 是不可变字节容器。它不接受字符串归一化，不尝试重排 JSON，也不重新序列化；`facts_sha256` 直接对原始 bytes 求 SHA-256。外层 `PreparedFacts` canonical 对象只编码 bytes 长度和 SHA，符合 RFC“外部原始字节”规则。

## 8. `CapturedFacts` 与 verified empty

公开 `CapturedFacts` 是通过 `try_new` 构造的不可变 acquisition 输出，因此公开 `capture_once` 不会泄漏私有签名。它包含：

```text
source_contract_id
source_contract_version
source_refs
canonical_facts: ExactBytes
provider_observed_at
facts_presence
model_output_refs
```

`FactsPresence` 不是裸 bool：

- `Present`：事实 schema 验证成功且不声明为空；
- `VerifiedEmpty(VerifiedEmptyEvidenceRef)`：限定 coverage 已验证成功且为空；

`CapturedFacts::try_new` 验证来源列表/时间列表/合同/重复项和 empty evidence 的局部一致性。W04 为既有不透明 `VerifiedEmptyEvidenceRef` 增加受校验生产构造器，要求 occurrence、source-contract ID、evidence SHA 与 verified-at 全部显式提供；它仍不是 transport authority。`PreparedFacts` 构造再验证 source-contract ID 等于 capture 私有的 catalog 预期值、version 等于 `RunContext`，并绑定 `run_context_sha256`。

公开构造器是 source adapter 的输入合同，不是发送或完成 authority。W06 catalog 决定哪个 adapter 可用于哪个 Unit；调用方不能通过它构造 `VerifiedTerminalRef` 或推进 completion。

acquisition 失败返回 `PreparationError::AcquisitionFailed { reason: ReasonCode }`，不返回空 `CapturedFacts`。`InputSourceUnavailable`、`InputSourceUnready`、`InputEvidenceInvalid` 等 reason 保持可供 W03 retry policy 判定。

## 9. `PreparedFacts` 与共享 snapshot

成功 capture 生成字段私有的 `PreparedFacts`：

| 字段 | 构造规则 |
| --- | --- |
| `run_context_sha256` | 从同一个 `RunContext` 规范化内容派生 |
| `source_contract_id` | 取自已校验的 `CapturedFacts` |
| `source_contract_version` | 必须与 `RunContext` 精确相等 |
| `source_refs` | 有序、唯一、不可变 |
| `canonical_facts` | exact bytes，不再序列化 |
| `facts_sha256` | 对 exact bytes 直接求 SHA-256；自身不进入自己的派生材料 |
| `provider_observed_at` | 与 source refs 一一对应，unknown 为 None |
| `verified_empty` | 只由 `FactsPresence::VerifiedEmpty` 投影 |
| `model_output_refs` | 一次捕获、有序、不可变 |

`PreparedFactsSnapshot` 内含：

```rust
Arc<PreparedFacts>
```

它只提供只读 `facts()` 和 `shares_instance_with()`。clone 只增加引用计数；没有 `Arc<Mutex<_>>`、interior mutability 或重新构建入口。W17 可把同一 snapshot 分发给 active 与 shadow project。

## 10. 单次捕获状态机

```text
Open
  └─ capture_once 开始前 → Capturing
       ├─ acquisition + validation 成功 → Sealed(snapshot)
       └─ acquisition/validation 失败   → Failed(error)

Capturing / Sealed / Failed
  └─ 任意 capture_once → 拒绝，不调用外部闭包，rejected_count += 1
```

状态转移在调用 acquisition 之前发生，因此 acquisition 即使 panic 也不会恢复为 `Open`；生产路径禁止 panic，本条仅防止 unwind 后意外二次外呼。W04 使用 `&mut self` 实现同进程串行所有权，不引入锁或全局并发。跨进程/重启的 durable single-capture 恢复属于 W07/W08/W11；重放必须读取首次冻结内容，不能新建一个“相同”的 capture 来绕过限制。

状态可观察接口只返回：`CaptureStateView::{Open,Capturing,Sealed,Failed}`、`attempt_count` 和 `rejected_count`。`attempt_count` 只在真正进入 acquisition 前从 0 变 1；拒绝调用不增加 attempt。

## 11. 公共接口轮廓

```rust
pub struct RunContext { /* private */ }
pub struct PreparedFacts { /* private */ }
pub struct CapturedFacts { /* private */ }
pub struct PreparedFactsSnapshot(Arc<PreparedFacts>);
pub struct PreparationCapture { /* private state */ }

impl PreparationCapture {
    pub fn context(&self) -> &RunContext;
    pub fn capture_once<F>(&mut self, acquire: F)
        -> std::result::Result<PreparedFactsSnapshot, PreparationError>
    where
        F: FnOnce(&RunContext)
            -> std::result::Result<CapturedFacts, PreparationError>;
    pub fn state(&self) -> CaptureStateView;
    pub fn attempt_count(&self) -> u64;
    pub fn rejected_count(&self) -> u64;
}
```

`CapturedFacts::try_new` 是公开、受校验的 adapter seam。`RunContextFactory` 的 catalog binding 装配入口和 `begin_capture` 保持 crate-private，避免 W06 前任意调用方自行声明生产注册；W06 再通过 catalog-owned facade 给 production adapter 提供合法 capture capability。

## 12. 错误与失败语义

`PushJobError` 增加精确构造错误：invalid date/Git SHA/schema、重复或错序 source、source time 不闭合、source contract mismatch、invalid verified-empty evidence、invalid model refs。

运行捕获使用独立 `PreparationError`：

- `AcquisitionFailed { reason }`：外部来源/模型/业务读取失败；
- `InvalidCapturedFacts(PushJobError)`：适配器输出违反合同；
- `AlreadyAttempted { state }`：能力已用过；
- acquisition panic 不被捕获并伪装成业务错误；若测试在外层捕获 unwind，状态保持 `Capturing`，仍不可重试。

错误不携带原始 payload、受保护模型引用内容或凭据。Display 只打印类型化原因，不打印 exact bytes。

## 13. TDD 验收矩阵

| 测试 | 必须证明 |
| --- | --- |
| `w04_run_context_golden_hash_is_stable` | 15 字段、trigger、null/数组规则形成固定 bytes/SHA；复用 W01 后 W01 golden 不变 |
| `w04_run_context_rejects_catalog_binding_mismatch` | Unit/producer/source version/build/catalog/generation 不能由 caller 任意漂移 |
| `w04_exact_bytes_hash_original_payload` | 空格、字段顺序、非 UTF-8 bytes 都按原字节求 SHA，不重写 |
| `w04_source_times_are_total_ordered_and_allow_unknown` | 与 source ref 一一对应；None 保留；缺失/重复/错序拒绝 |
| `w04_source_and_model_refs_are_frozen` | 顺序不变、重复拒绝、模型输入输出 hash/protected ref 一次冻结 |
| `w04_failure_cannot_be_labeled_verified_empty` | acquisition failure 不产生 PreparedFacts；verified empty 必须有 evidence |
| `w04_active_and_shadow_share_same_snapshot` | clone 后 `Arc::ptr_eq` 为 true，model refs 相同且无重算入口 |
| `w04_second_capture_is_rejected_before_external_call` | acquisition counter=1、attempt=1、rejected=1，第二闭包 counter 不增加 |
| `w04_failed_first_capture_is_also_single_use` | 首次失败后第二闭包不调用；原 typed reason 保留 |
| `w04_capture_unwind_does_not_reopen_capability` | unwind 后状态非 Open，后续闭包不调用 |
| `w04_has_no_runtime_wiring` | diff 不包含 monitor binary、notification、durable、config、Cargo feature 或 DB schema 改动 |

## 14. 提交与验证

按可回滚小提交执行：

1. W04 设计；
2. W04 实施计划；
3. RED：RunContext/canonical 合同测试；
4. GREEN：canonical 抽取与 RunContext；
5. RED：facts/snapshot/single-capture 测试；
6. GREEN：facts 与状态机；
7. review 修复、结果证据文档。

每个 GREEN 运行 `cargo test --lib monitor::push_job -- --nocapture`。最终 fresh 验证包括目标测试、rustdoc、`cargo check --lib`、targeted rustfmt、diff check 和 production-wiring path diff。全仓既有 fmt/Clippy/durable 并行测试异常继续按基线/目标改动判因，不得隐藏，也不得把无关失败误称 W04 成功。

## 15. 明确非目标

- W05 才定义 `SemanticProjection`、`PreparedPush`、首次渲染 exact bytes 和重放封存。
- W06 才读取机器 catalog，闭合 65 kind/102 producer/52 Unit 并创建正式 `RunContextFactory` binding。
- W07/W08/W11 才提供跨进程 durable capture/intent 恢复、outbox、lease/fence。
- W15 才实现 activation manifest generation 的运行校验。
- W17 才实现 old/new typed diff 与八类 shadow 副作用计数拒绝端口。
- W04 不迁移任何实际推送项，不改变 physical owner，不重启或替换生产 monitor。
