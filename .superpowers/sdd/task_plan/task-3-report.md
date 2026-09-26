# Task 3 — 独立连续 PaperLedger 与原子风控

状态：源码实现与限定验证完成，待独立 review；不代表 Task5/Task10 或生产验收完成。基线 `c792ec46cf5c3a0527c8ae6dde5a11640abc0ae9`；唯一需求为 `task-3-brief.md`，预检约束为 `task-3-preflight.md`。未部署、未读取或写入生产数据库、未替用户选定 seed/cutover。

## TDD 日志

### Slice 1 — seed/reopen

- RED：`cargo test --lib paper_ledger_seed_is_once_and_ignores_later_account_snapshots -- --nocapture`，exit 101；新 Interface `SeedManifest/PaperLedger/PaperCommand` 尚不存在；另有测试缺 `RunQueryDsl` 导入，一并修正。
- 实现：一次性显式 seed、manifest/epoch 绑定、不可变账户/事件、完整重放对账、私有 tempfile DB 重开。旧 `ledger` 更新不能改变新模拟余额。
- 首次 GREEN 尝试出现未声明 `chrono_tz`；改用明确 UTC+8 的 chrono FixedOffset，无新增依赖。
- GREEN：相同命令 exit 0，1 passed（3m42s 编译；测试 0.30s）。

### Slice 2 — 两买、部分卖、跨日估值

- RED：`cargo test --lib paper_ledger_two_buys_partial_sell_and_next_day_mark -- --nocapture`，exit 101；`ExecuteIntent/ValuationBatch`、Execute/Mark variants 与状态字段尚不存在。
- 实现：锁内取 projection、完整库存指纹与估值证据、冻结风险阈值与整数费用；T+1 FIFO 分摊买入费用；同事务兼容成交和不可变审计；head CAS。非成交无财务变化，sell 不受买入现金底或集中度门影响。
- 独立预期：98995 → 97790 → 98883.90 现金；费用 16.10；已实现 88.90；D1 收盘权益 100183.90，D0 收盘基准下日收益 -6.10。
- GREEN：相同命令 exit 0，1 passed（3m27s 编译；测试 0.26s）。

### Slice 3 — 幂等与不确定提交

- RED：`cargo test --lib paper_ledger_idempotency_ -- --nocapture`，exit 101；2 个真实行为失败：重开后同单返回 `VersionChanged`；提交后故障未返回 `CommitOutcomeUnknown`。0 passed / 2 failed。
- 实现：完整链验证后、版本/新鲜度前检查 command 与语义 plan；同业务意图返回冻结 receipt，改变数量/方向/价格冲突；Rejected 可明确新 attempt；终态计划部分唯一索引；commit 返回不确定时必须同身份恢复。
- GREEN：相同命令 exit 0，2 passed（3m40s 编译；测试 0.35s）。

### Slice 4 — 真实双连接额度与价格意图

- 首次 test harness 编译出现 scoped thread 捕获生命周期 E0373；改为显式 move + 借用各自 DB，未将此错误冒充业务 RED。
- RED：`cargo test --lib paper_ledger_concurrent_ -- --nocapture`，exit 101，1 passed / 1 failed。市场信号换 quote 后重放错误返回 IdentityConflict；同时，两独立 DatabaseManager/连接 + Barrier 的现金与集中度竞争测试已通过此前同事务实现（诚实记录为首跑 GREEN，不制造人工 RED）。
- 实现：`SignalQuoteMarketV1` 排除变化行情与时间，只绑定固定业务意图和 manifest/policy；`FixedSignalPriceV1` 保留价格身份。原成交回执继续冻结旧价格/费用。
- GREEN：相同命令 exit 0，2 passed（3m36s 编译；测试 0.60s）。

### Slice 5 — 原子失败、完整性与维护边界

- RED：`cargo test --lib paper_ledger_integrity_ -- --nocapture`，exit 101；显式缺缓存恢复 Interface 尚不存在（E0599）。
- 实现：分离 verified facts replay 与 cache 比较；只允许显式重建缺失 projection，拒绝覆盖已存在但不一致的 head；seed 排除负残差。
- GREEN：相同命令 exit 0，4 passed（3m45s 编译；测试 5.75s）。三个故障点（order_audit_chain INSERT、event INSERT、head UPDATE/CAS）均无孤立成交或审计；重试只记一份费用。另覆盖 append-only trigger、篡改事件/缓存拒绝、lock busy 分类、锁内新鲜度和取消。负残差测试随本组首个可运行版本即 GREEN，不将其记为已观察业务 RED。

### D01 owner ruling（主审确认）

D01 仅写候选池，不再使用时间戳 plan 直接模拟买入。tick/evening 共用持久 pushed_stocks row ID；物理旧 worker drain/activation 属 Task10。测试必须观察 D01 入池零直接成交和后续单 owner 同 row 一次成交。

### Slice 6 — 实际调用者与 epoch

- RED：`cargo test --lib paper_ledger_runtime_ -- --nocapture`，exit 101；生产 runtime adapter 不存在（E0433）。
- GREEN：相同命令 exit 0，1 passed（3m43s 编译；0.27s）。旧 Filled 原样保留、不导入新 inventory；两个显式 epoch 各自连续记账；同 plan 返回原成交价/费用；未 seed 不能执行。
- 接线回归：`cargo test --lib paper_runtime_ -- --nocapture`，exit 0，8 passed（3m46s 编译；0.29s）；原慢报价、取消/drain、panic 保留已成交和逐票继续扫描保持，新卖出夹具已改为真实私有 SQLite PaperLedger。
- D01 RED：`cargo test --bin monitor paper_ledger_d01_ -- --nocapture`，exit 101，1 failed（2m51s 编译）。旧直接模拟路径先要求真实账户 banner，导致独立账户中连候选也无法入池：`BR-134 complete account metrics are unavailable`。删除直接 simulate，仅保留持久候选记录；后续 tick/evening 以 row ID 成交。
- D01 GREEN 被最终 `cargo test --bin monitor paper_scan_runtime_tests -- --nocapture --test-threads=1` 覆盖：4 passed，包含该真实子进程行为测试；证明 D01 入池时零成交，tick 首次成交后即使清除 consumed 模拟崩溃仍只有一笔成交、一份佣金。
- 保留旧测试专用 fixture pipeline，不对生产编译提供旧裸余额写入口；public `simulate` 明确 fail-closed。`stock_position` 域的原账户参考未全局替换。

### CatalogV2 — 资格与迁移边界

- 首次 `cargo test --lib paper_ledger_catalog_v2_ -- --nocapture` 为测试 harness E0369（旧 enum 未实现 PartialEq）；改用 matches，不冒充业务 RED。
- RED：同命令 exit 101，0 passed / 1 failed；完整扩展被旧代次检查拒绝 `UnsupportedFutureGeneration { actual: 2, supported: 1 }`。
- 实现：新增 V2/PaperLedgerV1 精确对象、DDL、FK、index 与 trigger reference；V1 fixture/hash 不动。完整旧基态与已声明扩展但不完整的状态分别分类；旧 selection receipt 不得给新代次发 authority。
- 第一轮 GREEN 尝试：1 passed / 1 failed；三种扩展基态和 partial/extra/tamper 用例通过，旧 receipt 用例被其刚创建的私有 WAL fixture 身份变化挡住。按已有 fixture 模式，在连接关闭后清理该私有 fixture 的已知 sidecar；未放松生产 sidecar 检查。
- 最终：`cargo test --lib database::global_schema_ -- --nocapture --test-threads=1`，exit 0，48 passed / 2 ignored（供其他测试启动的子进程 helper）；12.99s。覆盖三份 frozen V1、全部扩展负路径、旧 receipt 拒绝、只读检查与进程锁。
- 普通 production `run_migrations` 不扩 paper 表；真实测试 process 可在隔离 fixture 上创建。DDL 是始终编译的 crate maintenance seam，非 cfg(test)，Task10 才在 lease/backup/资格流程下显式执行。

### 历史读取与费用余数收口

- RED：`cargo test --lib paper_ledger_history_ -- --nocapture`，exit 101；`read_at_version/effective_fills/unrealized_pnl` 公共行为尚不存在（E0599）。
- 实现：完整 head/链验证后按版本读，按绑定 epoch 输出同一事件链的 effective-fill seam；seed 冻结已校验的原审计链高水位。保留独立成本基准，不把用户历史浮亏变成新模拟交易亏损。
- GREEN：同命令 exit 0，4 passed（3m50s 编译，0.40s）。覆盖历史版本、未实现收益、手续费最后 lot 吸收余数、整数溢出回滚、NotFilled/Invalidated/Rejected 幂等、显式重试、Mark 恒等且无财务副作用，以及无现金账户降险卖出不受买入门阻拦。

### 调用者限定回归记录

- `cargo test --lib decision::intraday_monitor::tests -- --nocapture --test-threads=1` 首跑 exit 101，19 passed / 3 failed；三个旧 skip/debounce 夹具未提供新显式账户 binding。将这三个用例迁到 seeded binding，并以 panic-on-execute 证明过滤/防抖不会执行交易，不降低生产未 seed 时 fail-closed 的要求。
- `cargo test --lib trading::paper_ -- --nocapture --test-threads=1` 首轮 exit 0，95 passed（3m46s 编译，10.16s）；覆盖 ledger、原 FIFO、paper_trade、paper_engine 以及真实卖出慢报价/取消/drain/panic 保留已成交。用例中的 quote panic 为预期注入，测试正常通过。

### 自审修复 — adapter 恢复必须先于报价 I/O

- 自审发现：核心已先恢复幂等，但 runtime adapter 仍先采全账户 quotes。主审要求作为 Important 在本任务修复，不能只标 concern。
- RED：`cargo test --lib paper_ledger_runtime_terminal_recovery_ -- --nocapture`，exit 101，0 passed / 1 failed（3m43s 编译，0.33s）；已成交后重开 DB，另一个持仓的外部 source 强制失败且传入 quote 已过期，重复计划报 `TEST_CODE all external quotes unavailable`。
- 实现：Ledger 新只读 `recover_terminal`，与 Execute 复用同一 intent hash / 完整链校验 / 终态回放；adapter 在任何 quote/marks 采集之前调用。market price/time 不属于业务身份；数量冲突仍为 `IdentityConflict`。Rejected 不被误恢复为终态，后续新 attempt 仍走锁内重判。
- 首次 GREEN 编译尝试出现 E0308/E0599：局部补丁误改了同名 command 参数的位置；已修正，仅 replay lookup 改为 optional command，原 event hash 编码维持不变。该编译失败不计业务 RED。
- GREEN：同命令 exit 0，1 passed（3m46s 编译，0.34s）。验证重开、stale quote、另一个持仓 source 强制失败时零外部 quote 仍返回原成交价；改数量为 typed conflict；现金、费用与 head 无变化。

## 最终限定验证

| 命令 | 结果 | 覆盖 |
| --- | --- | --- |
| `cargo test --lib trading::paper_ -- --nocapture --test-threads=1` | exit 0；96 passed；10.39s | 最终 core/runtime 恢复修复、连续账户、并发、费用、FIFO/T+1、rollback、旧夹具与真实卖出 |
| `cargo test --lib decision::intraday_monitor::tests -- --nocapture --test-threads=1` | exit 0；22 passed；0.33s | 盘中/盘后策略、消费恢复、同 row 一次成交、skip/debounce |
| `cargo test --lib trading::risk_adapter -- --nocapture` | exit 0；23 passed | 既有 AccountMode/DataMode、现金底和集中度政策兼容 |
| `cargo test --lib database::global_schema_ -- --nocapture --test-threads=1` | exit 0；48 passed / 2 ignored；12.99s | frozen V1、精确 V2 与拒绝旧 receipt；后续修改未改变 catalog 行为 |
| `cargo test --bin monitor paper_scan_runtime_tests -- --nocapture --test-threads=1` | exit 0；4 passed；2m39s 编译、0.76s 测试 | D01 实际调用和两条 supervisor/drain 子进程测试；另保留 1 个原有结构断言，新行为测试没有以源码字符串代替 |

最终上述限定集合共 193 passed、2 个 helper ignored，无全量测试、release/check/clippy 重复编译；编译仍有 warning，未扩大为全仓 warning 清理。

`git diff --check` 与 `git diff --cached --check` 均 exit 0；新增模块和本次较大修改的相关文件已用限定文件列表 rustfmt。仅暂存 17 个 Task3 源码/测试文件及本报告，progress 只追加交接状态、未纳入本提交。

## 模块与迁移说明

- 深模块为 `trading::paper_ledger`：显式 Seed / Execute / Mark、当前与历史版本 read、effective-fill 读取、缺失 projection 的显式维护修复；调用者不能提供裸 cash / total / position_pct 授权交易。
- `paper_ledger_execution` 在同一 `BEGIN IMMEDIATE` 内读取已验证 projection、检查风险、写兼容成交/审计/事件并更新 CAS head。纯微元 checked 算术，费用沿用既有 `lot-rates-v1` Scenario，并非真实券商费用声明。
- `paper_ledger_runtime` 只负责显式 binding、锁外报价和回执适配；绑定值为 `PAPER_LEDGER_ACCOUNT_BINDING` 中的 account_id / epoch_id / manifest_hash JSON。没有配置时拒绝，绝不自动 seed。
- D01 模板只 append 候选；tick/evening 以持久 pushed_stocks row ID 生成同一个 plan。候选 consumed 恢复通过本 epoch 的 ledger-event/兼容成交映射，不读取未归属的旧成交。
- `paper_sell` 使用本 epoch 可卖 lots；锁内重验 T+1/数量/现金。旧 `aggregate_open_positions` 仅保留作 legacy 诊断，不再作为生产卖出库存授权。
- `paper_trade::simulate` 生产路径 fail-closed；旧带裸财务参数的 pipeline 仅保留在 `cfg(test)` 兼容夹具中。`stock_position` / position_tracker 的另一账户域与真实账户快照展示不全局替换。
- `paper_ledger_schema_v1` 的三张表、唯一终态 plan index、不可变 triggers 与 CatalogV2 共用同一 DDL 定义；普通生产启动不调用扩表。全局 schema owner 对 generation 2 返回 `CatalogV2RequalificationRequired`，旧 receipt 不得发新 authority。

## 运行边界

### Catalog ruling（主审确认）

保持 legacy/transitional/final 三份 frozen v1 reference/fixture/hash 不变。新增明确 `GlobalSchemaCatalogV2/PaperLedgerV1` 代次，只接受合法基态 + 完整且 exact DDL/FK/index/trigger 的 paper 扩展；partial、额外影子对象或 tamper 均拒绝。旧 receipt/hash 只证明旧代次，不授权扩展或自动迁移。Task 10 负责显式生产迁移、重资格与 seed/cutover。需要三种基态及扩展负路径的独立 catalog 测试。

### Price intent ruling（主审确认）

现有 tick/evening/paper_sell 使用 `SignalQuoteMarketV1`：业务身份为 epoch + producer/stable plan + code + side + qty + 冻结 policy/contract version，signal.price/quote/time 是 attempt evidence；仍以本次 signal.price 成交，不改 fill 模型。明确 `FixedSignalPriceV1` 才把价格纳入意图 hash。Filled/NotFilled/Invalidated 为 plan terminal；RejectedAttempt 可新 command 重判，同 command 恒等。

- 所有新增账户测试只使用 `DatabaseManager::open_isolated_for_test` + `TEST_CODE_*.db` 临时文件。
- 不运行真实回填、上线、seed、release、activation 或 monitor 重启。
- 历史 paper 记录保留；生产 seed 和 cutover 必须在 Task 10 获得批准输入后执行。

## 自审与未覆盖边界

1. **生产前置，不是已上线声明**：Task10 必须取得明确 seed 金额/持仓、残差选择、可卖证据及 cutover，drain 旧 owner，然后在 maintenance lease/backup 下调用编译进生产的 DDL seam，写新 whole-catalog receipt 并重资格，最后绑定 epoch/activation。当前旧 receipt 对 V2 返回显式不具资格；不能跳过此门直接启动写入。
2. **Task5 消费者接线**：effective-fill seam 已建在同一事件链，未自动裁定任何历史异常 fill；旧经济报告/绩效读者的 quarantine/correction 与 epoch 过滤仍由 Task5 完成。Execute 有完整新鲜估值；显式 Mark/收盘基准已实现但未另建生产定时 Mark owner，后续接线需选择 admitted-close 来源/调度 owner，缺前交易日基准时 daily_pnl 明确为 None。新生产 epoch 在这些后续任务闭合前不激活，避免新旧原始行混算。
3. **Minor：规模性能尚未验收**：当前 read/apply/recover 对该账户历史链进行完整回放/哈希校验，复杂度为 O(n)，部分入口会多次验证。优先保证事实与缓存一致，没有引入可绕过校验的快速路径；未做长期大账本压测，独立 review/Task10 决定部署前阈值与是否需要受信 checkpoint/index。
4. 未做真实 gRPC、通知发送、生产交易日窗口、release 构建或 monitor dry-run；本任务证据来自限定 Rust 行为测试与私有 SQLite / 子进程隔离。未将这些源码/夹具结果写成线上成交验收。

## 改动文件

- 新增 `src/trading/paper_ledger.rs`、`paper_ledger_execution.rs`、`paper_ledger_runtime.rs`、`paper_ledger_tests.rs`：单一账本、原子决策、入口适配与行为测试。
- 新增 `src/database/paper_ledger_schema_v1.rs`；修改 `database/mod.rs`、`global_schema_catalog_v1.rs`、`global_schema_v1.rs`、`order_audit.rs`：扩展定义/资格、测试私有 schema 安装与原审计高水位校验复用。
- 修改 `src/decision/intraday_monitor.rs`、`src/trading/paper_sell.rs`、`paper_sell_runtime_tests.rs`、`paper_trade.rs`、`paper_engine.rs`、`mod.rs`：实际买卖 owner 迁移、旧生产裸余额旁路关闭与夹具兼容。
- 修改 `src/bin/monitor/push_templates.rs`、`paper_scan_runtime_tests.rs`：D01 单 owner、隔离真实 monitor 调用/退出链回归。
- 报告为本文件；未修改三份 frozen V1 fixture、Cargo 依赖、运行配置、用户快照或生产数据文件。
