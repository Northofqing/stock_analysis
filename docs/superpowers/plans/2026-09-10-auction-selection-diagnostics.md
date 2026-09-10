# P-02 选集拒绝原因：真实调用链的结构化保留

日期：2026-09-10。状态：计划及只读预检准备完成，尚未派发、未实现。前置W18严格请求入口已在9f35ab1完成限定复审；主线文档提交后固定本任务BASE并交接唯一Rust/Cargo作者。本任务消除真实P-02选集错误字符串中的信息丢失，不替代完整影子adapter、来源认证或量比来源接入。

## 目标、依据与当前调用

本任务落实 [P-02 接线前置中的缺源/拒绝合同](../../push-system/p02-shadow-integration-handoff-2026-09-10.md)和[量比来源缺口](../../push-system/auction-source-evidence-gaps-2026-09-08.md)：真实空池、字段缺失/非法、因已通知产生空选集不能混为同一种领域完成状态。

已核对 `8dbd3ef` 中未改的相关源码：

- `src/bin/monitor/push_templates.rs:5964` 的 `prepare_auction_volume_snapshot` 返回 `Result<AuctionVolumeSnapshot, String>`；非空输入中缺量比、非法价格/涨跌幅及全部已通知都会落入同一个无有效行字符串。
- 同文件 `:6040` 的纯 loader seam 和 `:6059` 的真实 loader 都调用该函数；`:6024` 的 tick 保留原始股票和实际来源观察，但 snapshot 的错误信息已经丢失具体成因。
- `src/bin/monitor/main.rs:9734` 实际借用该 tick 的 snapshot，错误仅写诊断、不发送；持仓检查继续消费同批原始股票。错误类型的 Display 可以直接进入现有日志，不需要新增一次 provider 调用。
- `src/market_analyzer/limit_up.rs:81` 的 `LimitUpObservation::status` 基于原观察区分 Available/VerifiedEmpty；`:295` 的真实股票投影仍为 `volume_ratio: None`。本任务的选择器没有资格把裸空 Vec 或选集为空升级为 VerifiedEmpty。
- 旧 P-02 测试中既有纯准备断言，也有调用 dispatcher 日志的用例；本任务只运行下述已明确的纯测试，不宽泛运行所有 `p02` 或整个 monitor 测试集合。

## Global Constraints

- 仅在 `/Users/zhangzhen/Desktop/Quant/stock_analysis/.worktrees/push-reliability-20260905` 的 `codex/push-reliability-20260905` 开发。不访问根工作树、真实 `.env`/`data/**`、生产 DB/monitor/provider/sink/订单/PAM/凭据、远端 Git 或部署。
- 唯一实现者、唯一 Cargo 队列；不得与 W18 或其他任务同时写 Rust、编译。主线拥有文档、Git 和独立审查，实现者只拥有 Task 1 列出的文件，不做 Git 写入、不派子 agent。
- 不改过滤条件、排序、Top10、sentiment、模板字节、通知集合推进、发送/入池路径、共享原始股票及来源观察。不能为方便分类改变缺字段时的实际发送行为。
- 不补量比、不跨源 join、不重采集、不改历史 canonical/schema/八份冻结文档、不改 Cargo/RUSTFLAGS/target/incremental，不全文件或全仓格式化无关代码。
- 新类型只陈述本次输入的选择事实；不签发 VerifiedEmpty、NoData、Ready、Suppressed、完成证据或任何执行权限。不以测试成功替代真实来源/生产验收。

## Task 1: 结构化失败接入原选择器与 tick

### 文件与 interface

仅修改 `src/bin/monitor/push_templates.rs` 的 P-02 选择器、紧邻类型和同文件纯测试。`main.rs` 的既有 Display 日志调用与真实 loader 必须编译适配，但无需为本任务改 main、来源采集或 dispatcher。若实际发现必需改其他文件，先向主线交付具体接口原因，不扩散修改。

将 `prepare_auction_volume_snapshot` 及 `AuctionVolumeTickData.snapshot` 的错误由 String 改为 `AuctionVolumeSelectionError`。这是实际调用链的类型变更，不增加保留旧信息丢失返回值的并行“诊断版”选择器。旧调用方通过同一个入口得到成功快照或 typed 拒绝；两处 loader 都保留它，外层 provider/初始化失败的 `Result<_, String>` 不在本任务改写。

错误只有两种：`SourceRowsEmpty` 与 `NoEligibleUnnotifiedRows`；后者携带私有字段、只读 getter 的 `AuctionVolumeRejectionCounts`，不存股票代码、名称、原始数值或自由错误字符串。不要把“全部已通知”直接命名成 Suppressed。

计数必须在实际筛选同一批输入时计算，不重新加载，也不建立第二套与选中算法可能漂移的过滤规则：

| 字段 | 精确含义 |
| --- | --- |
| source_rows | 原始股票行数，含重复代码行，不按 code 去重 |
| valid_rows | 量比存在、有限且正，价格有限且正，涨跌幅有限的行数；尚未排除已通知 |
| notified_valid_rows | 上述 valid_rows 中 code 已在本 tick 起始 notified 集合的行数 |
| missing_volume_ratio_rows | volume_ratio 为 None 的行数 |
| invalid_volume_ratio_rows | volume_ratio 为 Some 但非有限或不大于 0 的行数 |
| invalid_price_rows | price 非有限或不大于 0 的行数 |
| invalid_change_pct_rows | change_pct 非有限的行数 |

缺陷计数可重叠：一行可以同时缺量比且价格/涨跌幅非法；不得假定所有计数之和等于 source_rows，不因该行已通知就隐去缺字段。`valid_rows - notified_valid_rows` 是 Top10 截断之前的可选行数。只在它为 0 且 source_rows 非零时返回携带计数的拒绝；source_rows 为零只返回 SourceRowsEmpty。

提供只读事实判断 `all_source_rows_valid_and_notified()`：仅 `source_rows > 0 && valid_rows == source_rows && notified_valid_rows == source_rows` 时为 true。混合“部分有效且已通知、部分缺字段/非法”必须为 false；这只是可计算的事实，不自动映射 JobDecision、重试或完成政策。

选择器成功结果保持原 tuple 全字段、原稳定量比降序、排除已通知后 Top10、sentiment 和 watch_status；不将失败计数写入模板、PushRecordMeta 或通知集合。禁止删除/放宽既有独立文案、记录、集合断言。

### 真实诊断与无副作用

实现稳定、安全的 Display/Debug。空源保留原“竞价量能涨停列表为空”提示；无可选行保留原拒绝前缀，并带上述固定字段名和数字，使 main 的现有日志实际显示缺量比、非法输入和全部有效行已通知的区别。不打印代码、名称、价格原值、URI 或其他任意输入文本。

纯准备和纯 loader 测试只用合成内存股票/HashSet，失败时不得修改 notified、原始股票或触发渲染/发送/写库。真实 loader 的 source_observation 保留逻辑不变；纯 seam 的 None 不能被描述为真实来源证据。

### 验收与命令

同一实际 interface 下新增 `p02_selection_` 前缀的纯测试，至少覆盖：

1. 原始空 Vec 与非空但全部缺量比分别返回精确不同 typed 错误，不以 `.is_err()` 代替分类和数字断言。
2. 所有行字段有效且全部已通知；完整计数和 `all_source_rows_valid_and_notified()` 为 true，输入集合不变。
3. 混合有效已通知和缺字段行、全部代码已通知但仍有非法字段；两者事实判断必须 false，保留真实缺陷计数。
4. 同一行有多个缺陷、缺量比与非法 Some 量比、NaN/正负无穷/零/负价格及涨跌幅边界；计数交叉项明确，有限负涨跌幅不是非法。
5. 成功样本混有已通知/非法行、相同量比及超过十个有效候选；逐项固定期望选择顺序、完整 tuple 和 sentiment；输入原始顺序和 notified 不变。不用新的选择器计算预期。在计数/成功用例中显式加入重复 code：证明逐行计数、已通知code对重复行逐行生效，以及未通知重复行仍按原稳定排序保留、不被去重。
6. 纯 loader 一次调用，错误分类穿透到 tick，失败选集仍保留同批原始股票；provider 外层 Err 仍按原合同返回，不伪装成 SourceRowsEmpty。
7. Display/Debug 字面量或固定字段断言及敏感股票名称/代码哨兵不泄露。裸空列表不生成观察对象或任何 NoData/VerifiedEmpty capability。

先取得接口/行为反例，缺类型编译 RED 与真实运行期行为 RED 分别报告，不制造错误实现补过程。完成后的必要验证：

- `cargo test --bin monitor p02_selection_ -- --test-threads=1`：实际非零测试数，完整捕获 stdout/stderr/退出码。
- `cargo test --bin monitor p02_preparation_rejects_non_finite_and_non_positive_market_values -- --test-threads=1`：原数值选择规则。
- `cargo test --bin monitor auction_volume_p02_preparation_contains_exact_message_records_and_codes -- --test-threads=1`：纯冻结准备的既有独立字节和记录/集合预期。
- `cargo test --bin monitor auction_volume_p02_preparation_comparison_detects_price_metric_and_code_changes -- --test-threads=1`：纯完整提案比较仍能检测隐藏差异。
- `cargo clippy --bin monitor --message-format=json`：完整输出，区分本批诊断和非目标诊断；没有相应 BASE 对照不得称非目标全部既有。
- 改动块的定向格式检查与 `git diff --check`；不得为通过格式检查重写巨大文件的无关区域。

以上 Rust 检查在同一源码固定后执行，不重复已经有效的同源码结果；不运行 dispatcher 测试、不启动真实 monitor，也不以虚构 transport 调用计数证明生产无效果。主线固定 BASE..SOURCE 独立 Spec/Quality 审查后，才关闭本任务。

## 后续依赖与回滚

完成后，P-02 真实 tick 能保留明确选择失败事实，便于完整 W17 adapter 按实际来源观察/政策区分缺输入与已通知；仍需真实量比/来源认证、可信注册与 RunContext/投影构造、同次完整业务提案比较、八类实际效果纳管及 W16 owner/fence。不得因此计为 Unit 已迁移。

回滚只撤销 typed 选择错误与紧邻测试，恢复原错误签名；不改数据或生产状态。新日志信息只含字段名和计数，没有新增持久存储合同。
