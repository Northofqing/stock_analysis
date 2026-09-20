# 推送 Foundation W06 实现与验证结果

**结论：** W06 已实现并验证 exact-byte、类型化、只读、双向闭合的 Machine Catalog 运行时注册表。运行时只能从仓库冻结的原始 JSON 字节建立 65 个 kind、102 个 producer、52 个 Migration Unit；SHA、头部、数量、状态、枚举外身份集合或任一双向关系发生漂移，都会在发布注册表前 fail closed。

**边界：** W06 只是目录解释与校验，不是生产激活。`PROVISIONAL` 和目录中的 `ACTIVE` 分类均不授予执行权；当前 caller、scheduler、provider、数据库、sink、activation generation 和 physical owner 没有接线。W07--W21、52 个 Migration Unit 的 shadow/晋级/观察/回滚仍未完成。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 权威基线与取舍

W06 只消费可机械验证的结构事实：

- `docs/push-system/push-capability-catalog.v1.json` exact bytes，SHA-256 为 `0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3`；
- 65 kind / 102 producer / 52 Unit；
- 当前 RFC 状态计数 36 `ACTIVE` / 22 `INACTIVE` / 5 `STARVED` / 2 `OPT-IN`；
- 92 个 enum-bound producer + 10 个 enum-external producer；
- kind、producer、Unit、completion owner、occurrence family、phase 的双向闭合关系。

状态计数采用当前 RFC 和机器目录冻结值，不采用蓝图较早版本的 37/24/2/2。蓝图继续约束分层、MigrationUnit、single owner 和 Foundation 零行为；机器可执行事实以当前 catalog/RFC 为准。目录中的 trigger/source/authority/policy 自然语言、known gaps、note 不进入运行授权，也不被代码猜测为 source/template/audience/policy version。

## 2. 提交与 TDD 证据

| 提交 | 内容 | 证据 |
| --- | --- | --- |
| `143ec8c` | W06 Machine Catalog 设计 | 冻结权威、闭合不变量、非目标和零接线边界 |
| `9be9008` | W06 逐测试计划 | 固定两轮 RED→GREEN、双轴 review、fresh 门禁 |
| `1effc1f` | bundled/query 合同 RED | 7 个目标诊断均为 `MachineCatalog`、状态或查询合同尚不存在 |
| `0d69d2d` | typed registry GREEN | exact-byte 加载、类型化 entry、索引和只读查询落地 |
| `a6d1271` | 闭合失败合同 RED | 16 个目标诊断均为待实现的闭合错误类型/关系 |
| `9d76241` | closure GREEN | 两组反向关系、owner/family/phase、成员与身份门禁落地 |
| `7acfe23` | review 修复 | header 分类、digest 常量处理、构造权限、enum 外精确集合、全量 round-trip 加固 |

两个 RED 都位于对应实现提交之前，没有用语法错误、坏环境或全仓既有失败冒充目标失败。

实现中首次最小 GREEN 编译暴露 `raw.units` 与真实字段 `raw.migration_units` 不一致（Rust `E0609`）。修复只替换该字段引用，随后目标测试 2/2 通过；错误发生在提交前，没有进入最终历史或扩大修改范围。

## 3. exact-byte 加载门

| 约束 | 实现证据 | 测试证据 |
| --- | --- | --- |
| 编译进 exact 原始字节 | `catalog.rs:14-17` 使用 `include_bytes!` 和固定 SHA | `tests.rs:2214-2227` 核对 schema、baseline、evidence ID 和 SHA |
| SHA 优先 | `catalog.rs:237-258` 先求 digest，不匹配立即返回 | `tests.rs:2344-2349` digest mismatch |
| 严格头部与数量 | `catalog.rs:260-297` 检查 JSON、schema、`PROVISIONAL`、65/102/52、typed Git SHA/证据 ID | `tests.rs:2350-2383` schema/status/count/status-count 反例 |
| 全部通过后一次发布 | `catalog.rs:299-350` 在局部解析、建索引、做 closure，最后才返回 `Ok(catalog)` | 所有 mutation 均只得到错误，不存在 partial registry |
| 外部不可自签目录 | `parse_v1_exact` 仅为 `pub(super)`（`catalog.rs:248-251`），公开入口只有 `bundled()` | 生产调用者不能传入“篡改内容自己的 SHA”绕过 authority |

`MachineCatalogError` 在 `catalog.rs:68-126` 只携带分类、安全 ID、count、SHA 或 JSON line/column。非法 JSON 测试植入 `catalog-secret`，Debug 输出不含正文（`tests.rs:2385-2392`）。

## 4. 类型化、不可变注册表

| 对象 | 字段与 getter | 不变量 |
| --- | --- | --- |
| `CatalogKindRegistration` | `catalog.rs:129-152` | typed `MonitorKind`、`PhaseEpic`、`CatalogStatus`、`ProducerId[]` |
| `CatalogProducerRegistration` | `catalog.rs:155-188` | typed ID、0/1 kind、非空 phase、family、owner、Unit |
| `CatalogUnitRegistration` | `catalog.rs:191-219` | typed Unit/owner、非空 producer/family/phase 集合 |
| `MachineCatalog` | `catalog.rs:221-234` | 私有 vectors + 私有 `BTreeMap` indexes；无 insert/remove/mutable getter |

状态是闭集：Machine Catalog 仅允许 `Provisional`，业务状态仅允许 `Active/Inactive/Starved/OptIn`（`catalog.rs:35-46`）。phase 严格映射盘前、集合竞价、盘中、盘后；未知 phase/status 由 `catalog.rs:813-844` 拒绝，反例见 `tests.rs:2623-2639`。

公开查询位于 `catalog.rs:353-424`：

- 按 kind、producer ID、Unit ID 查询；
- kind→producers、Unit→producers、producer→Unit；
- 单独枚举 `enum_external_producers()`；
- 返回借用或只读集合，不暴露注册表修改能力。

`tests.rs:2690-2718` 对全部 102 producer 和 52 Unit 做双向 round-trip，不只验证样例。

## 5. 精确基数和 enum-external 身份

固定常量位于 `catalog.rs:18-33`：65/102/52、10 个枚举外 producer，以及它们的完整身份集合。注册表发布前还验证：

- 65 个 kind 精确覆盖 `MonitorKind::ALL`，状态计数严格为 36/22/5/2（`catalog.rs:502-523`）；
- enum-external 不仅数量等于 10，身份集合也必须完全相等（`catalog.rs:429-451`）；
- 实际基线为 92 enum-bound + 10 enum-external（`tests.rs:2256-2265`）。

10 个独立入口是：

1. `chain-post-close-timer`
2. `chain-preopen-timer`
3. `cli-chain`
4. `cli-replay-force`
5. `cli-single-default`
6. `cli-single-lhb`
7. `cli-single-schedule`
8. `cli-summary-default`
9. `cli-summary-lhb`
10. `cli-summary-schedule`

`tests.rs:2722-2746` 把一个真实外部 producer 绑入 enum，同时释放一个 enum producer，使总数仍为 10；代码仍以 `EnumExternalIdentityMismatch` 拒绝，证明门禁不是弱数量检查。

## 6. 每一组关系的闭合证据

### 6.1 成员结构

`validate_closure` 在 `catalog.rs:426-500` 统一执行门禁。kind producer IDs 禁止重复但允许空（明确保留无 producer 的 inactive kind）；producer phases、Unit producers/families/phases 必须非空且成员唯一（453-493）。重复/空集合反例见 `tests.rs:2521-2564` 和 `2644-2686`。

### 6.2 kind ↔ producer

`catalog.rs:541-564` 遍历 `MonitorKind::ALL`，比较 kind 声明的 producer 集合与 102 producer 反向声明的 kind 集合，要求 set equality。删除 `PreopenNewsHot` 对 `p01-scheduled` 的正向引用会返回 `KindProducer` mismatch（`tests.rs:2396-2414`）；一个 producer 声明两个 kind 会在解析阶段拒绝（2416-2446）。

### 6.3 producer ↔ MigrationUnit

`catalog.rs:566-592` 先证明每个 producer 指向存在的 Unit，再逐 Unit 比较其 producer IDs 与 producer 反向 Unit ID 的集合。缺失 Unit 与反向漂移分别由 `tests.rs:2608-2622`、`2462-2475` 覆盖。

### 6.4 completion owner、occurrence family、phase

- 两个 Unit 不得共享 completion owner：`catalog.rs:525-539`；反例 `tests.rs:2521-2531`。
- 一个 Unit 的 owner 必须与全部成员 producer exact-match：`catalog.rs:594-616`；反例 `tests.rs:2476-2489`。
- Unit occurrence families 必须等于成员 producer family 的去重集合：`catalog.rs:618-628`；反例 `tests.rs:2490-2503`。
- Unit phases 必须等于成员 producer phase 的并集：`catalog.rs:630-640`；反例 `tests.rs:2504-2517`。

关系比较用集合，不把 JSON 数组顺序当 identity；getter 保留声明顺序供审计。

## 7. 代表路径证据

`tests.rs:2288-2324` 固定两条不同类型的路径：

```text
p01-scheduled
  → Some(PreopenNewsHot)
  → phase=盘前
  → occurrence=p01:{business_date}
  → MU-p01

chain-preopen-timer
  → monitor_kind=None（保持 enum-external）
  → phase=盘前
  → completion_owner=monitor_loop::CHAIN_PREOPEN_LAST[calendar_date]
  → MU-chain-preopen
```

第二条没有被伪装成第 66 个 `MonitorKind`，也没有被合并成模糊的“CLI/chain”概念入口。

## 8. 变异测试为何能证明 closure

`w06_mutated_catalog` 在 `tests.rs:2327-2341` 先修改 JSON，再为修改后的 exact bytes 重新计算 expected SHA，然后调用内部 parser。因此后续失败来自 schema/cardinality/type/relationship 门禁，而不是原始 catalog digest mismatch。

10 个 W06 测试覆盖：

1. exact authority、65/102/52、36/22/5/2；
2. 92+10、精确外部 ID、两条代表查询；
3. digest/schema/status/count/status-count/日志脱敏；
4. kind-producer、enum 外数量、producer kind cardinality；
5. Unit reverse/owner/family/phase；
6. duplicate owner、producer phase duplicate/empty；
7. unknown/duplicate/orphan kind/producer/Unit 与 unknown phase/status；
8. kind/Unit 成员 duplicate/empty；
9. 全 102 producer/52 Unit 查询 round-trip；
10. enum-external 数量不变但身份替换。

## 9. 双轴 review 及修复

| 发现 | 风险 | 修复 |
| --- | --- | --- |
| 头部值错误被标作 kind 错误 | 审计定位失真 | 新增 `CatalogEntity::Header`，baseline/digest/evidence 精确归类 |
| bundled digest 解析使用 `expect` | 常量错误可能 panic | 转为 typed `InvalidValue`，入口仍 fail closed |
| alternate bytes parser 曾对 crate 可见 | 外部模块可能以自算 SHA 自授权 | 收紧为 `pub(super)`，只有 push_job 内版本实现/测试可见 |
| enum 外只检查数量 | 10 个身份可被等量替换 | 固定完整 10-ID 集合并增加身份交换反例 |
| 查询只测代表项 | 可能遗漏长尾反向索引错误 | 增加全部 102 producer、52 Unit round-trip |

review 后没有遗留 W06 critical/high/medium finding。W04/W05 没有在此处接线，因为 catalog 尚无结构化 source contract/template/audience/policy version；这些绑定必须由后续 activation/assembly 明确提供，不能从自然语言描述推断。

## 10. Fresh 验证结果

| 命令/门禁 | 结果 |
| --- | --- |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS：51 passed / 0 failed / 2855 filtered；panic 文本来自既有 catch_unwind 反例，测试均为 ok |
| `cargo test --doc push_job` | PASS：3 passed / 0 failed / 17 filtered |
| `cargo check --lib` | PASS；84 条目标外既有 dead-code warning；无 W06 编译错误 |
| `cargo clippy --lib -- -A dead-code -D warnings` | 基线阻断：79 个目标外既有 lint，首个为 `src/data_gateway/futures_delivery.rs:15`；无 `push_job`/W06 error |
| `cargo clippy --lib -- -A dead-code` | PASS，exit 0；79 个 warning 均不在 `push_job` |
| 定向 `rustfmt --edition 2021 --check` | PASS：W01--W06 的 10 个目标文件 |
| architecture docs 五组验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| `git diff --check b595819..HEAD` | PASS |
| production-wiring relative diff | PASS：`src/bin/monitor`、`src/notification`、`src/durable_delivery`、`config`、`Cargo.toml`、`Cargo.lock` 均无 diff |

strict Clippy 的 79 个错误是本切片外旧基线，本文不把它误写成全仓 strict lint 全绿。

## 11. 生产观察与下一步

验证时生产进程仍是既有 `./target/release/monitor` PID 20162；两条既有 TCP 连接保持。TCP established 只能证明连接存在，不能证明业务消息已被接收或推送成功。W06 没有改变该进程、二进制、配置或生产数据库。

下一步是 W07：持久化 business intent/outbox，并把同一 intent 的 immutable payload drift 转成 `ResolutionRequired`。之后仍需 W08--W21 以及 52 个 Migration Unit 的逐项迁移和生产证据，才能宣称整体推送改造完成。
