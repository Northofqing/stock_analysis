# 迁移调查记录

## v13 实际边界

- `chain_post_close.v12.sql:6` 创建 `chain_post_close_macro_query_terminals`；`cause_kind` 与 cause-specific `CHECK` 都排除 `OperationRetired`。`chain_post_close.v12.sql:81` 的 dimension terminal 通过 `(intent_id,last_query_terminal_version)` 引用该表。终态表上另有 immutable update/delete triggers。
- `chain_post_close.v13.sql:286-297` 先撤销 12 个 Macro INSERT guards，`:299-1026` 重建它们；其中 `query_terminals_guard` 约在 `:780`，包含 `LocalRouteUnavailable` 只匹配真实断连的条件。v14 必须重建这 12 个 guard，使写入在新布局下准入；旧布局和旧行不能修改。
- `chain_post_close_macro_live.rs::initialize` 当前会存所有传入 request；driver 在 Local 可用时传入 `Gateway(5)` 请求。新计划可不传该请求，转而在同一事务写 `OperationRetired` terminal。恢复校验不能要求旧计划绝无 Gateway(5) 请求，因为旧计划已经持久记录它。
- `chain_post_close_macro_recovery.rs` 对 `LocalRouteUnavailable` 重建 `local_unavailable_outcome` 并要求 v3 计划真实断连；新 cause 必须独立重建固定 `GatewayError::retired_operation(EconomicCalendar-Jin10, Jin10)`，校验 `NotCalled` 与无 data begin，但不能把旧 request plan 的存在误判成新 RPC。
- 旧计划若已有 Gateway(5) attempt begin，不能生成 `NotCalled` 终态抹掉可能发生的调用；需要明确恢复策略，最少先 fail closed 并保留原有 Unknown/结果事实。

## SQLite 换表试验

用内存 SQLite 的最小父表/子表（子表一行 FK 引用父表）试验：`foreign_keys=ON`、事务内 `defer_foreign_keys=ON`、复制父表、DROP、RENAME 后 `foreign_key_check=[]`，但 `COMMIT` 仍报 `FOREIGN KEY constraint failed`。若在 `BEGIN IMMEDIATE` 前设 `foreign_keys=OFF`，事务内复制/换表并执行 `foreign_key_check=[]`，提交后设回 `foreign_keys=ON`，原引用行仍可联接且检查通过。这仅证明 SQLite 技术可行；正式迁移要在本仓同一事务内做 v13 catalog/事实验真、完整 FK 校验、v14 catalog 封印、失败回滚和旧行不可变检查。

更稳妥的候选设计是新增独立 v14 `OperationRetired` 终态表，不重建有外键引用的 v12 表，也不关闭外键。它仍要求 12 个 Macro guard 把新表纳入 run-version 冲突检查和 Gateway(5) 完成计数；恢复需把新表追加为第 13 个事实组，保持旧组索引不变。此方案与旧表并存，须通过混合旧/新事实的全链恢复测试后才能采用。

## 2026-09-25 实施与验证进度

- 已采用独立 v14 sidecar：旧 `chain_post_close_macro_query_terminals` 及其外键保持原状，v14 新表只接受 `Gateway(5)`、`OperationRetired`、`NotCalled` 和 `operation_retired` 审计。12 个 Macro guard 与 3 个 Models guard 已按新布局重新封印。
- 新 v14 Local 计划不再生成 `Gateway(5)` request plan；连接路线下在计划事务写退役终态。旧连接计划若只有 request plan 而没有 attempt，恢复时允许保留该请求事实并补退役终态；若已有 attempt，则不伪造 `NotCalled`，后续调用 fail closed。
- `v14_connected_local_retires_economic_without_rpc_and_reopens_terminal` 已通过：loopback 服务端只收到另外四个 Gateway 调用，退役终态及审计在重开数据库后可读。
- 排查中发现持久执行器将 LocalBridgeV1 新闻结果强制绑定到 ExternalV1 线身份；已按 profile 分支修正。原 v12 五 Gateway 确认后预算到期用例通过。原 v12 完整成功场景在当前机器上运行到 Web 研究阶段后耗尽固定 15 秒预算，不能作为本次迁移通过的证据。
- 含真实 v12 旧终态的 v12→v13→v14 迁移测试已通过：五条旧终态、运行头、StageFinal、五条审计及其审计链逐行保持，`foreign_key_check` 为零，重开后仍恢复原 EconomicCalendar 结果。v14 catalog 三条定向测试和未来版本拒绝测试也已通过。
- 2026-09-26：v13 已连接计划只有 Gateway(5) request plan、没有 attempt 的续跑用例通过。旧请求行保留，v14 补 `OperationRetired` / `NotCalled`，四个新闻源继续执行，旧 RPC 零调用，重开后终态可读。
- 2026-09-26：Web pace 夹具生成了真实 v12 dimension 行，外键指向旧 query terminal。用 `VACUUM INTO` 对暂停写入的数据库做一致性副本，再在副本上升级 v13、v14：旧请求、attempt、结果、终态、dimension、审计及审计链逐行保持，外键关联仍可联接，`foreign_key_check` 为零；原 pace 重开用例继续通过。
- 2026-09-26：v14 完整 Models/Search/Report 用例通过。四个新闻源加退役终态进入同一完整链，Models 三项效果与最终 artifact 持久化；重开后无模型或旧 EconomicCalendar 远程重放。共用夹具的 v13 用例复测也通过。
- `BusinessIntentStore::chain_post_close()` 仍为生产拒绝入口，当前没有自动升级旧库的生产接线；完整 v12 成功场景在本机触及固定 15 秒预算，不能作为 v12 完整报告成功的证据。
