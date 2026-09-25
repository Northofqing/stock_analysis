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
