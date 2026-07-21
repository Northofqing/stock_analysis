# 盘后复盘部分证据隔离与逐任务重试设计

**日期：** 2026-07-21  
**规则：** AGENTS 2.1、2.2、2.3、2.4、2.7、2.8、2.10；BR-049、BR-092、BR-093、BR-103、BR-104、BR-106、BR-108、BR-110、BR-116、BR-139、BR-140

## 1. 问题与实证

真实 `monitor --review` 已证明 BR-139 能进入严格盘后批次，但本轮结果为 `pushed=0/8`：

- R-02、R-05、R-06 当前缺少合同要求的完整真实证据，本来就是不可用能力；
- R-04 在 21:00 前只是数据等待，却被记作失败，日志中“由 monitor_loop 重调”也已因 BR-139 单 owner 改造而失效；
- A-01 只读取虚拟观察记录第一项，第一只股票的日 K 质检失败会拒绝整个 A-01；
- A-10、R-03 分别会被单个名称、产业链或日 K 缺失拖垮整个子任务；
- R-08 把公告列表源作为整个事件日历的必备前置，源失败时连其他独立完整组件也无法形成显式降级报告；
- 八个 dispatcher 都只返回 `bool`，聚合层无法区分等待、能力禁用、无数据、可重试失败和已经投递。

这是失败粒度和调度状态设计问题。修复不得把缺字段填零、放宽 K 线质量门或把不可用能力伪装成成功。

## 2. 方案比较与选择

### 方案 A：继续整批 fail-fast

优点是实现不变；缺点是任一候选或来源失败仍容易造成用户 0 消息，也无法正确安排 R-04。拒绝。

### 方案 B：逐任务、逐标的隔离（采用）

每个任务返回强类型结果；任务内部只让通过完整真实证据和质量门的标的参与计算，坏标的单独审计。调度器只重试仍未终结的任务，避免成功消息重复和永久禁用任务反复访问。该方案直接修复已观察到的失败面，且不降低数据红线。

### 方案 C：立即引入完整 capability graph 和 DataEnvelope

长期最完整，但会同时改造全部数据源、模板和调度边界，风险与验证面不适合本次 release monitor 恢复。作为后续架构项保留。

## 3. 强类型结果

新增可审计的任务结果：

```rust
enum ReviewTaskOutcome {
    Delivered { count: usize },
    NoData { reason: String },
    ExpectedWait { retry_at: NaiveTime, reason: String },
    Disabled { capability: String, reason: String },
    Failed { retryable: bool, reason: String },
}
```

语义固定如下：

- `Delivered`：真实 sink 已确认，才算用户可见成功；
- `NoData`：真实来源成功且完整，但按业务规则确实没有可报告对象；
- `ExpectedWait`：来源尚未到发布时间，到点后重试；
- `Disabled`：实现缺少合同要求的真实证据，禁止访问无关网络、禁止每分钟报错；
- `Failed`：来源、字段、质量门或 sink 失败；只有 `retryable=true` 才进入重试。

聚合审计必须分别输出各状态数量和任务名。CLI 的成功标准仍是至少一个 `Delivered`，不得把 `NoData`、`Disabled` 或等待伪装成推送成功。

## 4. 逐任务调度状态

BR-139 从“任一成功即提交整日完成”收紧为“逐任务提交终态”：

- `Delivered`、`NoData`、`Disabled` 在当日成为终态，不再重复执行；
- `ExpectedWait` 在 `retry_at` 前不执行，到点后重新进入 due；
- 可重试 `Failed` 使用固定有界退避：第一次失败后 1 分钟、第二次后 5 分钟、其后 15 分钟；避免持续请求故障源。不可重试失败成为当日终态并保留审计；
- 日期变化时清空任务状态；只使用真实交易日历；
- 已投递任务不会因其他任务失败而再次调用 sink。

19:00 批次与 21:00 龙虎榜因此可以由同一个 scheduler 负责，而不恢复第二个 `monitor_loop` owner。

## 5. 子任务数据隔离

### A-01 虚拟观察复盘

按快照记录顺序逐项验证 entry 字段、T+1 日期和 BR-092 日 K。单只股票失败记录脱敏的 code hash/source/reason 后继续；选择第一条完整且已到 T+1 的记录生成报告。全部失败才返回汇总 `Failed`，全部尚未到 T+1 返回 `NoData`。推送去重键使用真实记录 code，不再传空字符串。

K 线的异常涨跌幅、缺少 amount 或来源失败仍由原质量门拒绝；本设计不批准任何数值例外。

### A-10 催化复盘

真实 chain/rotation 数据必须具有与复盘日期一致的 as-of 证据。解析 rotation 时逐行隔离非法 code/name；主线候选缺名时只能从真实证券主数据补证，仍缺则排除。至少一个候选完整才渲染，并在审计中记录排除数；不能用代码或占位符冒充名称。

### R-03 产业链

持仓与自选逐票处理。产业链为空时仅可从真实行业映射补证；仍缺或日 K 失败则该票拒绝并继续。聚合只消费完整子集；发生排除时 `source_complete=false`，报告和审计明确为部分证据。完整子集没有当日涨停标的返回 `NoData`，不是伪造一条报告。

### R-08 事件日历

公告列表协议可增加仓库内已有、严格解析并保留 provenance 的真实备用协议；任一公告详情失败只拒绝该公告。公告组件失败时，其他组件只有在各自来源、时间和字段完整时才可独立形成报告，正文必须显示公告能力不可用。真实持仓必须经过 30 秒 freshness gate；旧持仓不能作为今日事件证据。

## 6. 当前不可快速启用的任务

- R-02 缺 main-flow、money-effect 和仓位涨跌停证据；在这些能力完成前直接返回 `Disabled`，不要先拉取全市场数据再失败。
- R-05 缺 signal → decision → order/fill → closed outcome 的真实关联，返回 `Disabled`。
- R-06 缺带版本的真实失败分类结果，返回 `Disabled`。
- R-04 在 21:00 前返回 `ExpectedWait`；21:00 后才访问真实龙虎榜源。

把永久不可用任务正确分类不是“修好功能”；它只是停止误报和无效重试。未来启用必须另有设计、真实来源和 Gate 证据。

## 7. 失败、审计与隐私

- 每个候选失败都要留任务、来源、时间、规则 ID、是否可重试和脱敏对象标识；
- 不在控制台、PR、测试夹具或运维报告中复制真实账户、持仓、证券列表、凭据、webhook 或消息正文；
- dispatcher audit 增加 `status/retryable/next_attempt`，原 `success` 仅由 `Delivered` 派生；
- 真实 sink 失败必须是 `Failed`，不得因渲染成功记为投递成功；
- 外部源全部不可用时允许 0 投递，但必须产生明确、可重试且可观察的系统状态，绝不凭空构造消息。

## 8. 测试与发布验收

必须先写 RED 测试覆盖：

- 强类型聚合对 Delivered/NoData/ExpectedWait/Disabled/Failed 的分类；
- 一项成功不终止其他待到点/可重试任务；
- A-01 第一条失败而后一条合规时继续，全部失败时汇总失败，且使用 code 去重；
- A-10 单个缺名不拖垮已验证候选，全部缺名失败，拒绝过期行；
- R-03 缺 sector 或 K 线按票隔离，部分证据标记 degraded；
- R-04 21:00 前等待、21:00 后 due；
- R-02/R-05/R-06 禁用时不访问数据源；
- R-08 主协议失败后真实备用协议成功、全部失败显式错误、单详情失败不丢其他合规公告。

最终 Gate：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`
- `bash tools/compliance/check.sh`
- `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json`
- `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`
- `cargo build --release --bin monitor`
- 合并 `master` 后单实例重启；脱敏验证 scheduler、逐任务状态、至少一项真实投递或明确的全部来源失败证据；48 小时累计从该版本启动时重新计时。

## 9. 旧模块关系与回滚

采用 BR-092 K 线质检、BR-103 账户 freshness、BR-110 dispatcher audit、BR-139 单 scheduler owner、现有 push governance 和 sink 确认。拒绝恢复 `monitor_loop` 的第二个 19:00/21:00 owner，也拒绝使用旧硬编码/空向量复盘。

回滚 BR-140 的 typed outcome、逐任务状态、隔离 loader 和测试提交即可；不删除任何数据库、持仓、虚拟观察、投递或事件审计数据。
